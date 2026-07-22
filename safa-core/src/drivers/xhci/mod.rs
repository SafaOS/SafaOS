use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{
    interrupts::IRQInfo,
    utils::{read_ref, write_ref},
};
use alloc::{collections::vec_deque::VecDeque, vec::Vec};
use regs::{CapsReg, XHCIDoorbellManager};
use rings::{command::XHCICommandRing, event::XHCIEventRing};

use crate::{
    arch::{with_interrupts, without_interrupts},
    debug,
    drivers::{
        interrupts::{self, IntTrigger, InterruptReceiver},
        keyboard::usb_kbd::USBKeyboard,
        pci::{AllocatedBar, PCICommandReg},
        usb_mouse::USBMouseDriver,
        xhci::{
            devices::XHCIDevice,
            extended_caps::XHCIUSBSupportedProtocolCap,
            regs::XHCIRegisters,
            rings::{
                transfer::XHCITransferRing,
                trbs::{
                    self, AddressDeviceCommandTRB, CmdResponseTRB, CompletionStatusCode,
                    ConfigureEndpointCommandTRB, DataStageTRB, EvaluateContextCMDTRB, EventDataTRB,
                    EventResponseTRB, PortStatusChangeTRB, SetupStageTRB, StatusStageTRB,
                    TRB_TYPE_ENABLE_SLOT_CMD, TransferResponseTRB, XHCIDeviceRequestPacket,
                },
            },
            usb::{GenericUSBDescriptor, UsbDeviceDescriptor},
            usb_device::USBDevice,
            usb_interface::USBInterface,
            utils::XHCIError,
        },
    },
    error,
    memory::frame_allocator::{self},
    process::current::kernel_thread_spawn,
    scheduler::wait_queue::WaitQueue,
    sleep_until,
    thread::Tid,
    utils::locks::{Mutex, RwLock, RwLockReadGuard, SpinLock},
    warn,
};

use super::pci::PCIDevice;
mod devices;
mod extended_caps;
mod regs;
mod rings;
mod usb;
pub mod usb_device;
mod usb_endpoint;
mod usb_interface;

pub mod usb_hid;
mod utils;

/// The maximum number of TRBs a CommandRing can hold
const MAX_TRB_COUNT: usize = 256;

fn handle_port_conn(xhci: &XHCI, port_index: u8, disconnected: bool) {
    let op_regs = unsafe { xhci.regs.as_mut_unchecked().operational_regs() };

    debug!(XHCI, "port {} resetting...", port_index);
    let reset_successful =
        unsafe { op_regs.reset_port(xhci.usb3_ports.contains(&port_index), port_index) };

    if reset_successful && !disconnected {
        if let Err(e) = xhci.setup_device(port_index) {
            error!(XHCI, "failed to connect port {}, err: {e}...", port_index);
        } else {
            debug!(XHCI, "port {} connected...", port_index);
        }
    }

    if disconnected {
        debug!(XHCI, "port {} disconnected...", port_index);
    }
}

fn handle_port_status_change(xhci: &XHCI, event: PortStatusChangeTRB) {
    let op_regs = unsafe { xhci.regs.as_mut_unchecked().operational_regs() };
    let port_index = event.parameter.port_index();
    let port_regs = unsafe { op_regs.port_registers(port_index) };
    let port_sc = read_ref!(port_regs.port_sc);

    let is_connected = port_sc.ccs();
    if port_sc.csc() {
        handle_port_conn(xhci, port_index, !is_connected);
    }
}

fn handle_status_change_thread(_: Tid, xhci: &XHCI) -> ! {
    let mut port_changes = VecDeque::new();
    loop {
        let mut new_port_changes = xhci.port_changes.lock();
        while let Some(change) = new_port_changes.pop() {
            port_changes.push_back(change);
        }
        drop(new_port_changes);

        while let Some(event) = port_changes.pop_front() {
            handle_port_status_change(xhci, event);
        }

        let new_port_changes = xhci.port_changes.lock();
        let pending_wait = xhci.other_wait_queue.prepare_wait();
        if !new_port_changes.is_empty() {
            continue;
        }
        drop(new_port_changes);
        pending_wait
            .enter_wait(XHCIWaitReason::PortStatusChange, None)
            .expect("Failed to wait for port status changes")
    }
}
fn on_interrupt_thread(_: Tid, xhci: &XHCI) -> ! {
    loop {
        let event_ring = unsafe { &mut *xhci.event_ring.get() };
        let mut events_pool = xhci.responses_manager.events.lock();

        event_ring.dequeue_events(|event| {
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
                        xhci.responses_manager
                            .add_command_response(&mut events_pool, res);
                    }
                    EventResponseTRB::TransferResponse(res) => {
                        let slot_id = res.cmd.slot_id();
                        if let Some(mut connected_devices) = xhci.connected_devices.try_write() {
                            let target_device = connected_devices
                                .iter_mut()
                                .find(|device| device.slot_id() == slot_id);

                            if let Some(target_device) = target_device {
                                // pass on the transfer event to the device
                                return target_device
                                    .on_event(&xhci.responses_manager, res.cmd.endpoint_id());
                            }
                        }

                        xhci.responses_manager
                            .add_transfer_response(&mut events_pool, res)
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

                        let mut curr_port_changes = xhci.port_changes.lock();
                        curr_port_changes.push(event);

                        let mut wait_queue = xhci.other_wait_queue.lock();
                        drop(curr_port_changes);
                        wait_queue.wake_equals(&XHCIWaitReason::PortStatusChange);
                    }
                }
            }
        });
        drop(events_pool);

        without_interrupts(|| unsafe {
            let pending_wait = xhci.interrupters_wait_queue.prepare_wait();
            if !(*xhci.event_ring.get()).is_empty() {
                return;
            }
            pending_wait
                .enter_wait((), None)
                .expect("Failed to wait for XHCI")
        })
    }
}

impl<'s> InterruptReceiver for XHCI<'s> {
    fn handle_interrupt(&self) -> bool {
        let regs = unsafe { self.regs.as_mut_unchecked() };
        // Defer work to another thread.
        self.interrupters_wait_queue
            .lock()
            .wake_n_on_condition(|_| true, 1);
        unsafe {
            // We only use interrupter 0 for now
            regs.acknowledge_irq(0);
        }

        true
    }
}

#[derive(Debug)]
struct XHCIResponsesPool {
    transfers: Vec<TransferResponseTRB>,
    commands: Vec<CmdResponseTRB>,
}

/// A safe communicator with XHCI Interrupts that can safely send requests and receive responses without deadlocking
#[derive(Debug)]
pub struct XHCIResponseQueue<'s> {
    events: Mutex<XHCIResponsesPool>,
    transfer_events_count: AtomicUsize,
    command_events_count: AtomicUsize,

    doorbell_manager: Mutex<XHCIDoorbellManager<'s>>,
    commands_ring: Mutex<XHCICommandRing<'s>>,
}

impl<'s> XHCIResponseQueue<'s> {
    pub fn new(
        doorbell_manager: XHCIDoorbellManager<'s>,
        commands_ring: XHCICommandRing<'s>,
    ) -> Self {
        Self {
            commands_ring: Mutex::new(commands_ring),
            doorbell_manager: Mutex::new(doorbell_manager),
            events: Mutex::new(XHCIResponsesPool {
                transfers: Vec::new(),
                commands: Vec::new(),
            }),
            transfer_events_count: AtomicUsize::new(0),
            command_events_count: AtomicUsize::new(0),
        }
    }

    fn add_command_response(&self, pool: &mut XHCIResponsesPool, response: CmdResponseTRB) {
        pool.commands.push(response);
        self.command_events_count.fetch_add(1, Ordering::Release);
    }

    fn add_transfer_response(&self, pool: &mut XHCIResponsesPool, response: TransferResponseTRB) {
        pool.transfers.push(response);
        self.transfer_events_count.fetch_add(1, Ordering::Relaxed);
    }

    unsafe fn wait_for_command_response(
        &self,
        cmds_len_before: usize,
    ) -> Result<CmdResponseTRB, XHCIError> {
        if !sleep_until!(200 ms, self.command_events_count.load(Ordering::Acquire) != cmds_len_before)
        {
            return Err(XHCIError::NoCommandResponse);
        }
        let response = self.events.lock().commands.drain(..).last().unwrap();

        Ok(response)
    }

    /// Enqieue a TRB command in the XHCI command ring, and rings the command doorbell, then returns the response TRB
    pub fn send_command(&self, trb: trbs::TRB) -> Result<CmdResponseTRB, XHCIError> {
        let mut doorbell = self.doorbell_manager.lock();
        let cmds_len_before = self.command_events_count.load(Ordering::Relaxed);

        self.commands_ring.lock().enqueue(trb);
        doorbell.ring_command_doorbell();

        let response = unsafe { self.wait_for_command_response(cmds_len_before) }?;

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
        let mut doorbell = self.doorbell_manager.lock();
        let transfer_events_before = self.transfer_events_count.load(Ordering::Acquire);

        doorbell.ring_control_endpoint_doorbell(transfer_ring.doorbell_id());

        if !sleep_until!(400 ms, self.transfer_events_count.load(Ordering::Acquire) != transfer_events_before)
        {
            return Err(XHCIError::NoTransferResponse);
        }

        let response = self.events.lock().transfers.drain(..).last().unwrap();
        self.transfer_events_count.fetch_sub(1, Ordering::Release);

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
        // We don't want to waste any frame allocator time here that is why we acquire a lock for each operation
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
            warn!(
                "XHCI failed to perform first transfer: {e}, if you are using qemu then this is expected"
            );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XHCIWaitReason {
    PortStatusChange,
}

// TODO: maybe stack interrupt stuff together in one struct behind a Mutex?
/// The main XHCI driver Instance
#[derive(Debug)]
pub struct XHCI<'s> {
    /// be careful using the registers everything there is unsafe
    regs: UnsafeCell<XHCIRegisters<'s>>,
    /// Only accessed by interrupts
    ///
    /// UnsafeCell because it is modified by hardware outside of the driver.
    event_ring: UnsafeCell<XHCIEventRing<'s>>,
    responses_manager: XHCIResponseQueue<'s>,
    /// A list of USB3 ports, all other ports are USB2
    usb3_ports: Vec<u8>,
    connected_devices: RwLock<Vec<USBDevice>>,

    irq_info: IRQInfo,

    /// XHCI's interrupt treads wait queues,
    interrupters_wait_queue: SpinLock<WaitQueue<2>>,
    other_wait_queue: Mutex<WaitQueue<2, XHCIWaitReason>>,
    port_changes: Mutex<Vec<PortStatusChangeTRB>>,
}

unsafe impl<'s> Send for XHCI<'s> {}
unsafe impl<'s> Sync for XHCI<'s> {}

impl<'s> XHCI<'s> {
    pub fn read_connected_devices(&self) -> RwLockReadGuard<'_, Vec<USBDevice>> {
        self.connected_devices.read()
    }

    /// A helper function to send an Enable Slot TRB Command to the XHCI controller, returns the slot id
    pub fn enable_device_slot(&self) -> Result<u8, XHCIError> {
        let trb = trbs::TRB::new(
            trbs::TRBCommand::default().with_trb_type(TRB_TYPE_ENABLE_SLOT_CMD),
            0,
            0,
        );

        let response = self.responses_manager.send_command(trb)?;
        Ok(response.cmd.slot_id())
    }

    pub fn address_device(&self, device: &XHCIDevice, bsr: bool) -> Result<(), XHCIError> {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();

        let trb = AddressDeviceCommandTRB::new(input_ctx_base_addr, bsr, slot_id, 0);
        self.responses_manager.send_command(trb.into_trb())?;
        Ok(())
    }

    pub fn evaluate_context(&self, device: &XHCIDevice) -> Result<(), XHCIError> {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();
        let trb = EvaluateContextCMDTRB::new(input_ctx_base_addr, slot_id);
        self.responses_manager.send_command(trb.into_trb())?;
        Ok(())
    }

    pub fn configure_endpoint(&self, device: &XHCIDevice) -> Result<(), XHCIError> {
        let slot_id = device.slot_id();
        let input_ctx_base_addr = device.input_ctx_base_addr();
        let trb = ConfigureEndpointCommandTRB::new(input_ctx_base_addr, slot_id);
        self.responses_manager.send_command(trb.into_trb())?;
        Ok(())
    }

    /// Checks all root hub ports for connected ports and handles them.
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
                handle_port_conn(self, i, !port_sc.ccs());
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
        device.fill_usb_descriptor(&self.responses_manager, &mut usb_descriptor, 8)?;
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
            &self.responses_manager,
            &mut usb_descriptor,
            usb_desc_header_len,
        )?;

        debug!(XHCI, "filled the usb descriptor: {:#x?}", usb_descriptor);
        let usb_configuration_desc =
            device.get_usb_configuration_descriptor(&self.responses_manager)?;

        let configuration_value = usb_configuration_desc.b_configuration_value as u16;
        debug!(
            XHCI,
            "configuring the device with value {}...", configuration_value
        );

        sync_inp_ctx!();
        device.set_configuration(&self.responses_manager, configuration_value)?;

        let manufacturer = device
            .get_string_descriptor(usb_descriptor.i_manufacturer, 0, &self.responses_manager)?
            .into_string();
        let product = device
            .get_string_descriptor(usb_descriptor.i_product, 0, &self.responses_manager)?
            .into_string();
        let serial_number = device
            .get_string_descriptor(usb_descriptor.i_serial_number, 0, &self.responses_manager)?
            .into_string();

        debug!(
            XHCI,
            "device {slot_id} has manufacturer: {manufacturer}, product: {product}, serial number: {serial_number}",
        );
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

        let mut connected_interfaces = Vec::new();

        // Attaches Drivers for this interface
        for interface_desc in interface_descriptors {
            let endpoints_descriptors = endpoint_descriptors
                .by_ref()
                .take(interface_desc.b_num_endpoints as usize);

            let endpoints = endpoints_descriptors.collect::<Vec<_>>();
            let mut interface = USBInterface::new(interface_desc, endpoints, slot_id)?;

            for endpoint in interface.endpoints_mut() {
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
                        device.set_protocol(&self.responses_manager, false)?;
                        interface.attach_driver::<USBKeyboard>();
                    }
                    2 => {
                        // sets the boot protocol
                        device.set_protocol(&self.responses_manager, false)?;
                        interface.attach_driver::<USBMouseDriver>();
                    }
                    _ => {}
                }
            }

            connected_interfaces.push(interface);
        }

        debug!(XHCI, "sending CONFIGURE_ENDPOINT command...");
        self.configure_endpoint(&device)?;

        for interface in &mut connected_interfaces {
            interface.start(&self.responses_manager);
        }

        let mut connected_devices = self.connected_devices.write();
        connected_devices.push(USBDevice::new(
            manufacturer,
            product,
            serial_number,
            usb_descriptor,
            slot_id,
            connected_interfaces,
        ));
        Ok(())
    }
}
impl<'s> PCIDevice for XHCI<'s> {
    const CLASS_SUBCLASS: (u8, u8) = (0xc, 0x3);
    const PROG_IF: Option<u8> = Some(0x30);

    fn create(mut info: super::pci::PCIDeviceInfo) -> Result<Self, &'static str> {
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
        let (allocated_bars, virt_base_addr, _) = AllocatedBar::allocate_bars::<6>(&"XHCI", &*bars);
        let AllocatedBar::Memory(_, _) = allocated_bars[0] else {
            unreachable!("XHCI Base bar should always be a memory bar")
        };

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
            .get_msix_cap(&*allocated_bars)
            .map(|msix| msix.into_irq_info())
            .unwrap();

        let this = XHCI {
            event_ring: UnsafeCell::new(event_ring),
            responses_manager: xhci_queue_manager,
            regs: UnsafeCell::new(xhci_registers),
            connected_devices: RwLock::new(Vec::new()),
            interrupters_wait_queue: SpinLock::new(WaitQueue::new()),
            other_wait_queue: Mutex::new(WaitQueue::new()),
            port_changes: Mutex::new(Vec::new()),
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
        // FIXME: More errors.
        Ok(this)
    }

    fn start(&'static self) -> bool {
        without_interrupts(|| {
            let irq_info = self.irq_info.clone();

            interrupts::register_irq(irq_info, IntTrigger::Edge, self);
            let int_tid = kernel_thread_spawn(
                on_interrupt_thread,
                self,
                Some(crate::thread::ContextPriority::High),
                None,
            )
            .expect("Failed to create XHCI Interrupt thread");

            let hotreload_tid = kernel_thread_spawn(
                handle_status_change_thread,
                self,
                Some(crate::thread::ContextPriority::High),
                None,
            )
            .expect("Failed to create XHCI Interrupt thread");

            let regs = unsafe { self.regs.as_mut_unchecked() };
            let op_regs = unsafe { regs.operational_regs() };
            let usbsts_before = read_ref!(op_regs.usbstatus);
            let usbcmd_before = read_ref!(op_regs.usbcmd);
            unsafe {
                regs.start();
            }
            let usbsts_after = read_ref!(op_regs.usbstatus);
            let usbcmd_after = read_ref!(op_regs.usbcmd);
            debug!(
                XHCI,
                "Started, usbsts before {:?} => usbsts after {:?}, usbcmd before {:?} => usbcmd after {:?}, interrupt thread: {}, hot-reload thread: {}",
                usbsts_before,
                usbsts_after,
                usbcmd_before,
                usbcmd_after,
                int_tid,
                hotreload_tid,
            );
        });

        with_interrupts(|| self.prob());
        true
    }
}
