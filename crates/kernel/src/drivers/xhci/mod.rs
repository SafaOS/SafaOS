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
        keyboard::usb_kbd::USBKeyboard,
        pci::PCICommandReg,
        xhci::{
            devices::XHCIDevice,
            extended_caps::XHCIUSBSupportedProtocolCap,
            regs::{OperationalRegs, XHCIRegisters},
            rings::{
                transfer::XHCITransferRing,
                trbs::{
                    self, AddressDeviceCommandTRB, CmdResponseTRB, CompletionStatusCode,
                    ConfigureEndpointCommandTRB, DataStageTRB, EvaluateContextCMDTRB, EventDataTRB,
                    EventResponseTRB, PortStatusChangeTRB, SetupStageTRB, StatusStageTRB,
                    TransferResponseTRB, XHCIDeviceRequestPacket, TRB_TYPE_ENABLE_SLOT_CMD,
                },
            },
            usb::{GenericUSBDescriptor, UsbDeviceDescriptor},
            usb_interface::USBInterface,
            utils::XHCIError,
        },
    },
    error,
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
mod usb_endpoint;
mod usb_interface;

pub mod usb_hid;
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
                        let slot_id = res.cmd.slot_id();
                        if let Some(mut connected_interfaces) = self.connected_interfaces.try_lock()
                        {
                            let target_interface = connected_interfaces
                                .iter_mut()
                                .find(|interface| interface.slot_id() == slot_id);

                            if let Some(target_interface) = target_interface {
                                // pass on the transfer event to the interface
                                target_interface.on_event(&self.manager_queue);
                                return;
                            }
                        }

                        self.manager_queue.add_transfer_response(res)
                    }
                    EventResponseTRB::PortStatusChange(event) => {
                        let code = event.status.completion_code();
                        let port_index = event.parameter.port_index();

                        debug!(
                            XHCI,
                            "port status change for port: {} with code {:?} ({:#x})",
                            port_index,
                            code,
                            code as u8,
                        );
                        self.manager_queue.add_port_status_change_event(
                            unsafe { self.regs.as_mut_unchecked().operational_regs() },
                            event,
                        );
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
    fn thread_name(&self) -> &'static str {
        "XHCI_POLL"
    }

    fn poll(&self) {
        let regs = unsafe { self.regs.as_mut_unchecked() };

        while let Some(event) = self.manager_queue.try_pop_port_connection_event() {
            let op_regs = unsafe { regs.operational_regs() };
            debug!(XHCI, "port {} resetting...", event.port_index);
            let reset_successful = unsafe {
                op_regs.reset_port(
                    self.usb3_ports.contains(&event.port_index),
                    event.port_index,
                )
            };

            if reset_successful && !event.disconnected {
                if let Err(e) = self.setup_device(event.port_index) {
                    error!(
                        XHCI,
                        "failed to connect port {}, err: {e}...", event.port_index
                    );
                } else {
                    debug!(XHCI, "port {} connected...", event.port_index);
                }
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

    pub fn add_port_status_change_event(
        &self,
        op_regs: &mut OperationalRegs,
        event: PortStatusChangeTRB,
    ) {
        let port_index = event.parameter.port_index();
        let port_regs = unsafe { op_regs.port_registers(port_index) };
        let port_sc = read_ref!(port_regs.port_sc);
        if port_sc.csc() {
            self.add_port_connection_event(port_index, !port_sc.ccs());
        }
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

    unsafe fn wait_for_command_response(
        &self,
        cmds_len_before: usize,
    ) -> Result<CmdResponseTRB, XHCIError> {
        let commands = unsafe { self.commands.as_mut_unchecked() };

        // FIXME: could this be optimized away, maybe i should use atomics?
        // FIXME: handle timeout instead of panicking
        if !sleep_until!(200 ms, commands.len() != cmds_len_before) {
            return Err(XHCIError::NoCommandResponse);
        }

        let response = commands.pop().unwrap();

        Ok(response)
    }

    /// Enqieue a TRB command in the XHCI command ring, and rings the command doorbell, then returns the response TRB
    pub fn send_command(&self, trb: trbs::TRB) -> Result<CmdResponseTRB, XHCIError> {
        let requester = self.requester_lock.lock();
        let cmds_len_before = unsafe { self.commands.as_ref_unchecked().len() };

        self.commands_ring.lock().enqueue(trb);
        self.doorbell_manager.lock().ring_command_doorbell();

        let response = unsafe { self.wait_for_command_response(cmds_len_before) }?;
        drop(requester);

        let code = response.status.code();
        if code != CompletionStatusCode::Success {
            return Err(XHCIError::CommandNotSuccessful(code));
        }

        Ok(response)
    }

    pub fn start_ctrl_ep_transfer(
        &self,
        transfer_ring: &XHCITransferRing,
    ) -> Result<TransferResponseTRB, XHCIError> {
        let requester = self.requester_lock.lock();
        let transfer_events = unsafe { self.transfer_events.as_mut_unchecked() };
        let transfers_len_before = transfer_events.len();

        self.doorbell_manager
            .lock()
            .ring_control_endpoint_doorbell(transfer_ring.doorbell_id());

        // FIXME: could this be optimized away, maybe i should use atomics?
        // FIXME: handle timeout instead of panicking
        if !sleep_until!(400 ms, transfer_events.len() != transfers_len_before) {
            return Err(XHCIError::NoTransferResponse);
        }

        let response = transfer_events.pop().unwrap();
        drop(requester);
        let code = response.status.completion_code();

        if code != CompletionStatusCode::Success {
            return Err(XHCIError::TransferNotSuccessful(code));
        }

        Ok(response)
    }

    /// performs a HOST->DEVICE no data control transfer on a `device`
    pub fn send_no_data_request_packet(
        &self,
        device: &mut XHCIDevice,
        packet: XHCIDeviceRequestPacket,
    ) -> Result<(), XHCIError> {
        let transfer_ring = device.transfer_ring();
        // Setup Stage
        let mut setup_stage = SetupStageTRB::new(packet, 0, 0);
        setup_stage.status.set_trb_transfer_length(8);
        setup_stage.info.set_ioc(false);
        setup_stage.info.set_idt(true);
        // No data stage
        setup_stage.info.set_trt(0);

        let mut status_stage = StatusStageTRB::new(0);
        status_stage.cmd.set_ioc(true);
        status_stage.cmd.set_dir_in(true);
        // don't chain an event
        status_stage.cmd.set_chain(false);

        transfer_ring.enqueue(setup_stage.into_trb());
        transfer_ring.enqueue(status_stage.into_trb());

        self.start_ctrl_ep_transfer(transfer_ring)?;
        Ok(())
    }
    pub fn send_request_packet(
        &self,
        device: &mut XHCIDevice,
        packet: XHCIDeviceRequestPacket,
        output: &mut [u8],
    ) -> Result<(), XHCIError> {
        let frame = frame_allocator::allocate_frame().ok_or(XHCIError::OutOfMemory)?;

        let (descriptor_buffer, descriptor_buffer_addr) =
            self::utils::allocate_buffers_frame::<u8>(frame, 0, 256);

        let (_, transfer_status_buffer_addr) = self::utils::allocate_buffers_frame::<u32>(
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
        if let Err(e) = self.start_ctrl_ep_transfer(transfer_ring) {
            warn!("XHCI failed to perform first transfer: {e}, if you are using qemu then this is expected");
        }

        let mut status_stage = StatusStageTRB::new(0);

        status_stage.cmd.set_ioc(true);
        status_stage.cmd.set_dir_in(false);
        // chain an event
        status_stage.cmd.set_chain(false);

        // enqueues the STATUS stage and the event stage
        transfer_ring.enqueue(status_stage.into_trb());

        // transfers the STATUS
        if let Err(e) = self.start_ctrl_ep_transfer(transfer_ring) {
            error!(
                "XHCI failed to transfer a request packet to device with slot {} and port {}, err: {e:?}",
                device.slot_id(),
                device.port_id(),
            );
            frame_allocator::deallocate_frame(frame);
            return Err(e);
        }

        // copy the output
        output.copy_from_slice(&descriptor_buffer[..output.len()]);
        frame_allocator::deallocate_frame(frame);
        Ok(())
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
    /// A list of USB3 ports, all other ports are USB2
    usb3_ports: Vec<u8>,
    // TODO: maybe it'd be first if we don't loop through all interfaces to figure out which one has the slot id
    connected_interfaces: Mutex<Vec<USBInterface>>,

    irq_info: IRQInfo,
}

unsafe impl<'s> Send for XHCI<'s> {}
unsafe impl<'s> Sync for XHCI<'s> {}

impl<'s> XHCI<'s> {
    /// A helper function to send an Enable Slot TRB Command to the XHCI controller, returns the slot id
    pub fn enable_device_slot(&self) -> Result<u8, XHCIError> {
        let trb = trbs::TRB::new(
            trbs::TRBCommand::default().with_trb_type(TRB_TYPE_ENABLE_SLOT_CMD),
            0,
            0,
        );

        let response = self.manager_queue.send_command(trb)?;
        Ok(response.cmd.slot_id())
    }

    pub fn address_device(&self, device: &XHCIDevice, bsr: bool) -> Result<(), XHCIError> {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();

        let trb = AddressDeviceCommandTRB::new(input_ctx_base_addr, bsr, slot_id, 0);
        self.manager_queue.send_command(trb.into_trb())?;
        Ok(())
    }

    pub fn evaluate_context(&self, device: &XHCIDevice) -> Result<(), XHCIError> {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();
        let trb = EvaluateContextCMDTRB::new(input_ctx_base_addr, slot_id);
        self.manager_queue.send_command(trb.into_trb())?;
        Ok(())
    }

    pub fn configure_endpoint(&self, device: &XHCIDevice) -> Result<(), XHCIError> {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();
        let trb = ConfigureEndpointCommandTRB::new(input_ctx_base_addr, slot_id);
        self.manager_queue.send_command(trb.into_trb())?;
        Ok(())
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

    /// Setups and initializes a USB Device with the port id `port_index` + 1
    /// you can find the steps done here at 4.3 of the XHCI Specification
    pub fn setup_device(&self, port_index: u8) -> Result<(), XHCIError> {
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

        let slot_id = self.enable_device_slot()?;
        debug!(XHCI, "slot {slot_id} was chosen for port {port_index}");

        let device_context_base = devices::allocate_device_ctx(context_sz_64bytes);
        unsafe {
            regs.set_dcbaa_entry(slot_id, device_context_base);
        }

        let mut device = XHCIDevice::create(context_sz_64bytes, port_index, slot_id, port_speed)?;
        // Configure and enable the control endpoint, with an initial size
        device.configure_ctrl_ep_input_ctx(max_initial_packet_size);

        // First address device with BSR=true, essentially blocking the SET_ADDRESS request,
        // but still enables the control endpoint which we can use to get the device descriptor.
        // Some legacy devices require their descriptor to be read before sending them a SET_ADDRESS command.
        self.address_device(&device, true)?;

        let mut usb_descriptor: UsbDeviceDescriptor = unsafe { core::mem::zeroed() };
        // get the actual max packet size
        device.fill_usb_descriptor(&self.manager_queue, &mut usb_descriptor, 8)?;
        debug!(
            XHCI,
            "filled the first 8 bytes of a usb descriptor: {:#x?}", usb_descriptor
        );

        // configures with the actual size
        let max_packet_size = usb_descriptor.b_max_packet_size_0 as u16;
        device.configure_ctrl_ep_input_ctx(max_packet_size);

        if max_packet_size != max_initial_packet_size {
            self.evaluate_context(&device)?;
        }

        /// syncs from the DCBAA to the input device context
        macro_rules! sync_inp_ctx {
            () => {
                unsafe {
                    let dest_input_device_ctx = device.get_input_device_ctx();
                    let src_out_device_ctx = regs.get_dcbaa_entry_as_ptr(device.slot_id());
                    dest_input_device_ctx.copy_from(src_out_device_ctx, 1);
                }
            };
        }

        // address device with bsr=false
        self.address_device(&device, false)?;

        // read the full descriptor
        let usb_desc_header_len = usb_descriptor.header.b_length as usize;
        device.fill_usb_descriptor(
            &self.manager_queue,
            &mut usb_descriptor,
            usb_desc_header_len,
        )?;

        debug!(XHCI, "filled the usb descriptor: {:#x?}", usb_descriptor);
        let usb_configuration_desc =
            device.get_usb_configuration_descriptor(&self.manager_queue)?;

        let configuration_value = usb_configuration_desc.b_configuration_value as u16;
        debug!(
            XHCI,
            "configuring the device with value {}...", configuration_value
        );

        sync_inp_ctx!();
        device.set_configuration(&self.manager_queue, configuration_value)?;

        let descriptors_iterator = usb_configuration_desc.into_iterator();

        let mut interface_descriptors = Vec::new();
        let mut endpoint_descriptors = Vec::new();

        for descriptor in descriptors_iterator {
            debug!(XHCI, "{descriptor:#x?}");
            match descriptor {
                GenericUSBDescriptor::Interface(int) => interface_descriptors.push(int),
                GenericUSBDescriptor::Endpoint(endpoint) => endpoint_descriptors.push(endpoint),
                _ => {}
            }
        }

        let mut endpoint_descriptors = endpoint_descriptors.into_iter();

        // Disables the control endpoint because it wouldn't be used anymore, and the Configure Endpoint command requires it to be off
        device.disable_ctrl_endpoint();

        let mut connected_interfaces = self.connected_interfaces.lock();
        let connected_interfaces_start_index = connected_interfaces.len();

        // Attaches Drivers for this interface
        for interface_desc in interface_descriptors {
            let endpoints_descriptors = endpoint_descriptors
                .by_ref()
                .take(interface_desc.b_num_endpoints as usize);

            let endpoints = endpoints_descriptors.collect::<Vec<_>>();
            let mut interface = USBInterface::new(interface_desc, endpoints, slot_id)?;

            for endpoint in interface.endpoints() {
                unsafe {
                    device.configure_ep_input_ctx(endpoint);
                }
            }

            let interface_desc = interface.desc();

            // currently only works with HID Boot protocol interfaces
            if interface_desc.b_interface_class == 0x3 && interface_desc.b_interface_subclass == 0x1
            {
                match interface_desc.b_interface_protocol {
                    1 => {
                        // sets the boot protocol
                        device.set_protocol(&self.manager_queue, false)?;
                        interface.attach_driver::<USBKeyboard>();
                    }
                    _ => {}
                }
            }

            connected_interfaces.push(interface);
        }

        debug!(XHCI, "sending CONFIGURE_ENDPOINT command...");
        self.configure_endpoint(&device)?;

        for interface in &mut connected_interfaces[connected_interfaces_start_index..] {
            interface.start(&self.manager_queue);
        }

        Ok(())
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
            connected_interfaces: Mutex::new(Vec::new()),
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
        crate::serial!("XHCI responded with enabling slot {test:#?}\n");
        true
    }
}
