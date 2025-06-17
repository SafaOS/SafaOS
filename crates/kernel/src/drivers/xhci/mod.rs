use core::cell::UnsafeCell;

use super::{
    interrupts::IRQInfo,
    utils::{read_ref, write_ref},
};
use alloc::vec::Vec;
use regs::{CapsReg, XHCIDoorbellManager};
use rings::{command::XHCICommandRing, event::XHCIEventRing};

use crate::{
    arch::{disable_interrupts, enable_interrupts, paging::current_higher_root_table},
    debug,
    drivers::{
        driver_poll::{self, PolledDriver},
        interrupts::{self, IntTrigger, InterruptReceiver},
        pci::PCICommandReg,
        xhci::{
            devices::XHCIDevice,
            extended_caps::XHCIUSBSupportedProtocolCap,
            regs::XHCIRegisters,
            rings::{
                transfer::XHCITransferRing,
                trbs::{
                    self, AddressDeviceCommandTRB, CmdResponseTRB, CompletionStatusCode,
                    DataStageTRB, EventDataTRB, EventResponseTRB, PortStatusChangeTRB,
                    SetupStageTRB, StatusStageTRB, TransferResponseTRB, XHCIDeviceRequestPacket,
                    TRB_TYPE_ENABLE_SLOT_CMD,
                },
            },
            usb::UsbDeviceDescriptor,
        },
    },
    memory::{
        frame_allocator,
        paging::{EntryFlags, PAGE_SIZE},
    },
    sleep_until,
    utils::locks::Mutex,
    warn,
};

use super::pci::PCIDevice;
mod devices;
mod extended_caps;
mod regs;
mod rings;
mod usb;
mod utils;

/// The maximum number of TRBs a CommandRing can hold
const MAX_TRB_COUNT: usize = 256;

impl<'s> InterruptReceiver for XHCI<'s> {
    fn handle_interrupt(&self) {
        let regs = unsafe { self.regs.as_mut_unchecked() };
        let events = self.event_ring.lock().dequeue_events();

        for event in events {
            if let Some(response_event) = event.into_event_trb() {
                match response_event {
                    EventResponseTRB::CommandCompletion(res) => {
                        debug!(
                            XHCI,
                            "command completed with code {:?} ({:#x}), slot: {}",
                            res.status.code(),
                            res.status.code() as u8,
                            res.cmd.slot_id(),
                        );
                        self.manager_queue.add_command_response(res)
                    }
                    EventResponseTRB::TransferResponse(res) => {
                        debug!(
                            XHCI,
                            "transfer completed with code {:?} ({:#x}), slot: {}",
                            res.status.completion_code(),
                            res.status.completion_code() as u8,
                            res.cmd.slot_id(),
                        );
                        self.manager_queue.add_transfer_response(res)
                    }
                    EventResponseTRB::PortStatusChange(event) => {
                        debug!(
                            XHCI,
                            "port status change for port: {} with code {:?} ({:#x})",
                            event.parameter.port_index(),
                            event.status.completion_code(),
                            event.status.completion_code() as u8,
                        );
                        self.manager_queue.add_port_status_change_event(event);
                    }
                }
            }
        }

        unsafe {
            // We only use interrupter 0 for now
            regs.acknowledge_irq(0);
        }
    }
}

impl<'s> PolledDriver for XHCI<'s> {
    fn poll(&self) {
        let regs = unsafe { self.regs.as_mut_unchecked() };

        if let Some(event) = self.manager_queue.try_pop_port_connection_event() {
            let op_regs = unsafe { regs.operational_regs() };
            debug!(XHCI, "port {} resetting...", event.port_index);
            let reset_successful = unsafe {
                op_regs.reset_port(
                    self.usb3_ports.contains(&event.port_index),
                    event.port_index,
                )
            };

            if reset_successful && !event.disconnected {
                self.setup_device(event.port_index);
                debug!(XHCI, "port {} connected...", event.port_index)
            }

            if event.disconnected {
                debug!(XHCI, "port {} disconnected...", event.port_index);
            }
        }
    }
}

/// A port connection or disconnection event
pub struct XHCIPortConnectionEvent {
    pub port_index: u8,
    pub disconnected: bool,
}

/// A safe communicator with XHCI Interrupts that can safely send requests and receive responses without deadlocking
#[derive(Debug)]
pub struct XHCIResponseQueue<'s> {
    // Only 1 interrupter may hold the lock
    // and Only 1 Reader may hold the lock (requester)
    // the idea is we might have a reader and writer at the same time but not 2
    // the reader has previously requested the writer to write so it is aware of it, and the writer will never remove
    interrupter_lock: Mutex<()>,
    requester_lock: Mutex<()>,

    commands: UnsafeCell<Vec<CmdResponseTRB>>,
    transfer_events: UnsafeCell<Vec<TransferResponseTRB>>,
    port_queue: UnsafeCell<Vec<PortStatusChangeTRB>>,
    port_connection_queue: UnsafeCell<Vec<XHCIPortConnectionEvent>>,

    doorbell_manager: Mutex<XHCIDoorbellManager<'s>>,
    commands_ring: Mutex<XHCICommandRing<'s>>,
}

impl<'s> XHCIResponseQueue<'s> {
    pub fn new(
        doorbell_manager: XHCIDoorbellManager<'s>,
        commands_ring: XHCICommandRing<'s>,
    ) -> Self {
        Self {
            interrupter_lock: Mutex::new(()),
            requester_lock: Mutex::new(()),
            commands_ring: Mutex::new(commands_ring),
            doorbell_manager: Mutex::new(doorbell_manager),
            commands: UnsafeCell::new(Vec::new()),
            transfer_events: UnsafeCell::new(Vec::new()),
            port_queue: UnsafeCell::new(Vec::new()),
            port_connection_queue: UnsafeCell::new(Vec::new()),
        }
    }

    pub fn add_command_response(&self, response: CmdResponseTRB) {
        let interrupter = self.interrupter_lock.lock();
        unsafe {
            self.commands.as_mut_unchecked().push(response);
        }
        drop(interrupter);
    }

    pub fn add_transfer_response(&self, response: TransferResponseTRB) {
        let interrupter = self.interrupter_lock.lock();
        unsafe {
            self.transfer_events.as_mut_unchecked().push(response);
        }
        drop(interrupter);
    }

    pub fn add_port_status_change_event(&self, event: PortStatusChangeTRB) {
        let interrupter = self.interrupter_lock.lock();
        unsafe {
            self.port_queue.as_mut_unchecked().push(event);
        }
        drop(interrupter);
    }

    pub fn add_port_connection_event(&self, port_index: u8, is_disconnected: bool) {
        let interrupter = self.interrupter_lock.lock();
        unsafe {
            self.port_connection_queue
                .as_mut_unchecked()
                .push(XHCIPortConnectionEvent {
                    port_index,
                    disconnected: is_disconnected,
                });
        }
        drop(interrupter);
    }

    pub fn try_pop_port_connection_event(&self) -> Option<XHCIPortConnectionEvent> {
        let lock = self.requester_lock.try_lock();
        let results = unsafe { self.port_connection_queue.as_mut_unchecked().pop() };
        drop(lock);
        results
    }

    unsafe fn wait_for_command_response(&self, cmds_len_before: usize) -> CmdResponseTRB {
        let commands = unsafe { self.commands.as_mut_unchecked() };

        // FIXME: could this be optimized away, maybe i should use atomics?
        // FIXME: handle timeout instead of panicking
        if !sleep_until!(200 ms, commands.len() != cmds_len_before) {
            panic!("XHCI timeout while waiting for response after 200ms",);
        }

        let response = commands.pop().unwrap();

        response
    }

    /// Enqieue a TRB command in the XHCI command ring, and rings the command doorbell, then returns the response TRB
    pub fn send_command(&self, trb: trbs::TRB) -> CmdResponseTRB {
        let requester = self.requester_lock.lock();
        let cmds_len_before = unsafe { self.commands.as_ref_unchecked().len() };

        self.commands_ring.lock().enqueue(trb);
        self.doorbell_manager.lock().ring_command_doorbell();

        let response = unsafe { self.wait_for_command_response(cmds_len_before) };
        drop(requester);
        response
    }

    pub fn start_ctrl_ep_transfer(
        &self,
        transfer_ring: &XHCITransferRing,
    ) -> Option<TransferResponseTRB> {
        let requester = self.requester_lock.lock();
        let transfer_events = unsafe { self.transfer_events.as_mut_unchecked() };
        let transfers_len_before = transfer_events.len();

        self.doorbell_manager
            .lock()
            .ring_control_endpoint_doorbell(transfer_ring.doorbell_id());

        // FIXME: could this be optimized away, maybe i should use atomics?
        // FIXME: handle timeout instead of panicking
        if !sleep_until!(400 ms, transfer_events.len() != transfers_len_before) {
            warn!("XHCI: timeout while waiting for Transfer TRB response after 400ms");
            return None;
        }

        let response = transfer_events.pop().unwrap();
        drop(requester);

        if response.status.completion_code() != CompletionStatusCode::Success {
            warn!("XHCI: Transfer Response TRB resulted from ringing the doorbell {} unsuccessful, code: {:?}", transfer_ring.doorbell_id(), response.status.completion_code());
            return None;
        }

        Some(response)
    }

    pub fn send_request_packet(
        &self,
        device: &mut XHCIDevice,
        packet: XHCIDeviceRequestPacket,
        output: &mut [u8],
    ) -> bool {
        let frame = frame_allocator::allocate_frame().unwrap();

        let (descriptor_buffer, descriptor_buffer_addr) =
            self::utils::allocate_buffers_frame::<u8>(frame, 0, 256);

        let (transfer_status_buffer, transfer_status_buffer_addr) =
            self::utils::allocate_buffers_frame::<u32>(
                frame,
                descriptor_buffer.len().next_multiple_of(16),
                1,
            );

        let transfer_ring = device.transfer_ring();

        // Setup Stage
        let mut setup_stage = SetupStageTRB::new(packet, 0, 0);
        setup_stage.status.set_trb_transfer_length(8);
        setup_stage.info.set_ioc(false);
        setup_stage.info.set_trt(3);
        // Data Stage
        let mut data_stage = DataStageTRB::new(descriptor_buffer_addr, 0);
        data_stage.parameter.set_td_size(0);
        data_stage
            .parameter
            .set_trb_transfer_len(output.len() as u32);
        data_stage.cmd.set_idt(false);
        data_stage.cmd.set_ioc(false);
        data_stage.cmd.set_dir_in(true);
        // chain the event
        data_stage.cmd.set_chain(true);

        // the event data stage (invokes an event)
        let mut first_event_data_stage =
            EventDataTRB::new(transfer_status_buffer_addr.into_raw() as u64, 0);
        first_event_data_stage.cmd.set_ioc(true);
        first_event_data_stage.cmd.set_chain(false);

        // first transfer the SETUP and DATA
        transfer_ring.enqueue(setup_stage.into_trb());
        transfer_ring.enqueue(data_stage.into_trb());
        transfer_ring.enqueue(first_event_data_stage.into_trb());
        // Transfer SETUP and DATA stages
        // FIXME: fails on qemu because it excepts a STATUS first which is a bug, so we don't return failure here
        // there is probably an alternative to using this such as chaining an event after status
        self.start_ctrl_ep_transfer(transfer_ring);

        let mut status_stage = StatusStageTRB::new(0);

        status_stage.cmd.set_ioc(false);
        status_stage.cmd.set_dir_in(false);
        // chain an event
        status_stage.cmd.set_chain(true);

        transfer_status_buffer[0] = 0;
        // invokes an event
        let mut second_event_data_stage =
            EventDataTRB::new(transfer_status_buffer_addr.into_raw() as u64, 0);
        second_event_data_stage.cmd.set_chain(false);
        second_event_data_stage.cmd.with_ioc(true);

        // enqueues the STATUS stage and the event stage
        transfer_ring.enqueue(status_stage.into_trb());
        transfer_ring.enqueue(second_event_data_stage.into_trb());

        // transfers the STATUS
        if self.start_ctrl_ep_transfer(transfer_ring).is_none() {
            warn!(
                "XHCI failed to transfer a request packet to device with slot {} and port {}",
                device.slot_id(),
                device.port_id()
            );
            frame_allocator::deallocate_frame(frame);
            return false;
        }

        // copy the output
        output.copy_from_slice(&descriptor_buffer[..output.len()]);
        frame_allocator::deallocate_frame(frame);
        true
    }
}

// TODO: maybe stack interrupt stuff together in one struct behind a Mutex?
/// The main XHCI driver Instance
#[derive(Debug)]
pub struct XHCI<'s> {
    /// be careful using the registers everything there is unsafe
    regs: UnsafeCell<XHCIRegisters<'s>>,
    /// Only accessed by interrupts
    event_ring: Mutex<XHCIEventRing<'s>>,
    manager_queue: XHCIResponseQueue<'s>,
    // TODO: fully implement
    /// A list of USB3 ports, all other ports are USB2
    usb3_ports: Vec<u8>,

    irq_info: IRQInfo,
}

unsafe impl<'s> Send for XHCI<'s> {}
unsafe impl<'s> Sync for XHCI<'s> {}

impl<'s> XHCI<'s> {
    /// A helper function to send an Enable Slot TRB Command to the XHCI controller, returns the slot id
    pub fn enable_device_slot(&self) -> u8 {
        let trb = trbs::TRB::new(
            trbs::TRBCommand::default().with_trb_type(TRB_TYPE_ENABLE_SLOT_CMD),
            0,
            0,
        );

        let response = self.manager_queue.send_command(trb);
        response.cmd.slot_id()
    }

    pub fn address_device(&self, device: &XHCIDevice, bsr: bool) {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();

        let trb = AddressDeviceCommandTRB::new(input_ctx_base_addr, bsr, slot_id, 0);
        let _ = self.manager_queue.send_command(trb.into_trb());
    }

    /// Checks all root hub ports for connected ports and adds them to the port connection queue
    pub fn prob(&self) {
        let regs = unsafe { self.regs.as_mut_unchecked() };
        let caps = unsafe { regs.captabilities() };
        let op_regs = unsafe { regs.operational_regs() };
        // Resettng all the root hub ports
        // TODO: detect connections
        for i in 0..caps.max_ports() {
            let port_regs = unsafe { op_regs.port_registers(i) };
            let port_sc = read_ref!(port_regs.port_sc);

            if port_sc.ccs() && port_sc.csc() {
                self.manager_queue
                    .add_port_connection_event(i, !port_sc.ccs());
            }
        }
    }

    pub fn setup_device(&self, port_index: u8) {
        let regs = unsafe { self.regs.as_mut_unchecked() };
        let cap_regs = unsafe { regs.captabilities() };
        let op_regs = unsafe { regs.operational_regs() };
        let port_regs = unsafe { op_regs.port_registers(port_index) };
        let context_sz_64bytes = cap_regs.context_sz_64bytes();

        let port_sc = read_ref!(port_regs.port_sc);
        let port_speed = port_sc.port_speed();
        let max_initial_packet_size = port_speed.max_control_transfer_initial_packet_size();

        debug!(
            XHCI,
            "setting up device at port: {port_index}, with speed: {port_speed:?} ({:#x}), context size 64 byte {context_sz_64bytes}",
            port_speed as u8
        );

        let slot_id = self.enable_device_slot();
        debug!(XHCI, "slot {slot_id} was chosen for port {port_index}");

        let device_context_base = devices::allocate_device_ctx(context_sz_64bytes);
        unsafe {
            regs.set_dcbaa_entry(slot_id, device_context_base);
        }

        let mut device = XHCIDevice::create(context_sz_64bytes, port_index, slot_id, port_speed);
        // Configure and enable the control endpoint
        device.configure_ctrl_ep_input_ctx(max_initial_packet_size);

        // First address device with BSR=1, essentially blocking the SET_ADDRESS request,
        // but still enables the control endpoint which we can use to get the device descriptor.
        // Some legacy devices require their descriptor to be read before sending them a SET_ADDRESS command.
        self.address_device(&device, true);

        let mut usb_descriptor: UsbDeviceDescriptor = unsafe { core::mem::zeroed() };
        // get the actual max packet size
        device.fill_usb_descriptor(&self.manager_queue, &mut usb_descriptor, 8);
        debug!(
            XHCI,
            "filled the first 8 bytes of a usb descriptor: {:#x?}", usb_descriptor
        );
    }
}
impl<'s> PCIDevice for XHCI<'s> {
    fn class() -> (u8, u8, u8) {
        (0xc, 0x3, 0x30)
    }

    fn create(mut info: super::pci::PCIDeviceInfo) -> Self {
        // Collect extended captability information
        let mut pci_caps = info.caps_list();
        let mut usb3_ports = Vec::new();

        while let Some(protocol_cap) =
            unsafe { pci_caps.find_next_transmute::<XHCIUSBSupportedProtocolCap>() }
        {
            if protocol_cap.major_version() == 3 {
                for port in
                    protocol_cap.first_compatible_port()..=protocol_cap.last_compatible_port()
                {
                    usb3_ports.push(port);
                }
            }
        }

        // Map and enable the XHCI PCI Device
        let general_header = info.unwrap_general();
        write_ref!(
            general_header.common.command,
            PCICommandReg::BUS_MASTER | PCICommandReg::MEM_SPACE
        );

        let bars = info.get_bars();
        let (base_addr, _) = bars[0];
        let virt_base_addr = base_addr.into_virt();

        unsafe {
            for (bar_base_addr, bar_size) in bars {
                let page_num = bar_size.div_ceil(PAGE_SIZE);
                current_higher_root_table()
                    .map_contiguous_pages(
                        bar_base_addr.into_virt(),
                        bar_base_addr,
                        page_num,
                        EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE,
                    )
                    .expect("failed to map the XHCI");
            }
        }
        // Create the XHCI Driver
        let caps_ptr = virt_base_addr.into_ptr::<CapsReg>();
        let caps_regs = unsafe { &mut *caps_ptr };

        let runtime_regs = unsafe { &mut *caps_regs.runtime_regs_ptr() };
        let interrupter = unsafe { &mut *runtime_regs.interrupter_ptr(0) };

        let command_ring = XHCICommandRing::create(MAX_TRB_COUNT);
        let mut event_ring = XHCIEventRing::create(MAX_TRB_COUNT, interrupter);

        let mut xhci_registers = unsafe { XHCIRegisters::new(caps_regs) };
        unsafe {
            xhci_registers.reconfigure(&mut event_ring, &command_ring);
        }

        let doorbell_manager =
            XHCIDoorbellManager::new(caps_regs.doorbells_base(), caps_regs.max_device_slots());

        let xhci_queue_manager = XHCIResponseQueue::new(doorbell_manager, command_ring);
        // FIXME: switch to MSI if not available
        let irq_info = info
            .get_msix_cap()
            .map(|msix| msix.into_irq_info())
            .unwrap();

        let this = XHCI {
            event_ring: Mutex::new(event_ring),
            manager_queue: xhci_queue_manager,
            regs: UnsafeCell::new(xhci_registers),
            usb3_ports,
            irq_info,
        };
        unsafe {
            debug!(
                XHCI,
                "Created\n{}\n{}\nUSB 3 ports: {:?}",
                this.regs.as_ref_unchecked().captabilities(),
                this.regs.as_mut_unchecked().operational_regs(),
                this.usb3_ports
            );
        }
        this
    }

    fn start(&'static self) -> bool {
        unsafe {
            disable_interrupts();
        }
        let irq_info = self.irq_info.clone();

        interrupts::register_irq(irq_info, IntTrigger::Edge, self);
        driver_poll::add_to_poll(self);

        let regs = unsafe { self.regs.as_mut_unchecked() };
        let op_regs = unsafe { regs.operational_regs() };
        let usbsts_before = read_ref!(op_regs.usbstatus);
        let usbcmd_before = read_ref!(op_regs.usbcmd);
        unsafe {
            regs.start();
            self.prob();
        }
        let usbsts_after = read_ref!(op_regs.usbstatus);
        let usbcmd_after = read_ref!(op_regs.usbcmd);
        debug!(
            XHCI,
            "Started, usbsts before {:?} => usbsts after {:?}, usbcmd before {:?} => usbcmd after {:?}", usbsts_before, usbsts_after, usbcmd_before, usbcmd_after
        );

        unsafe {
            enable_interrupts();
        }

        let test = self.enable_device_slot();
        crate::serial!("XHCI responded with enabling slot {test}\n");
        true
    }
}
