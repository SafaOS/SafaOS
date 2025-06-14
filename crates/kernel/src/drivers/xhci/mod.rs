use core::cell::UnsafeCell;

use super::{
    interrupts::IRQInfo,
    utils::{read_ref, write_ref},
};
use alloc::vec::Vec;
use regs::{CapsReg, XHCIDoorbellManager};
use rings::{XHCICommandRing, XHCIEventRing};

use crate::{
    arch::{disable_interrupts, enable_interrupts, paging::current_higher_root_table},
    debug,
    drivers::{
        driver_poll::{self, PolledDriver},
        interrupts::{self, IntTrigger, InterruptReceiver},
        pci::PCICommandReg,
        xhci::{
            extended_caps::XHCIUSBSupportedProtocolCap,
            regs::XHCIRegisters,
            trb::{
                CmdResponseTRB, EventResponseTRB, PortStatusChangeTRB, TransferResponseTRB,
                TRB_TYPE_ENABLE_SLOT_CMD,
            },
        },
    },
    memory::paging::{EntryFlags, PAGE_SIZE},
    time,
    utils::locks::Mutex,
};

use super::pci::PCIDevice;
mod extended_caps;
mod regs;
mod rings;
mod trb;
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
            regs.acknowledge_irq(0);
        }
    }
}

impl<'s> PolledDriver for XHCI<'s> {
    fn poll(&self) {
        let regs = unsafe { self.regs.as_mut_unchecked() };

        if let Some(event) = self.manager_queue.try_pop_port_connection_event() {
            let op_regs = unsafe { regs.operational_regs() };
            debug!(XHCIRegisters, "port {} resetting...", event.port_index);
            op_regs.reset_port(&self.usb3_ports, event.port_index);
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

    unsafe fn wait_for_command_response(
        &self,
        cmds_len_before: usize,
        timeout: u64,
    ) -> CmdResponseTRB {
        let commands = unsafe { self.commands.as_mut_unchecked() };

        let start_time = time!();
        // FIXME: could this be optimized away, maybe i should use atomics?
        while commands.len() == cmds_len_before {
            let now = time!();
            // FIXME: handle timeouting instead of panicking?
            if (now >= start_time + timeout) && commands.len() == cmds_len_before {
                panic!(
                    "XHCI timeout while waiting for response after {}ms",
                    now - start_time
                );
            }

            core::hint::spin_loop();
        }

        let response = commands.pop().unwrap();

        response
    }

    pub fn send_command(&self, trb: trb::TRB) -> CmdResponseTRB {
        let requester = self.requester_lock.lock();
        let cmds_len_before = unsafe { self.commands.as_ref_unchecked().len() };

        self.commands_ring.lock().enqueue(trb);
        self.doorbell_manager.lock().ring_command_doorbell();

        let response = unsafe { self.wait_for_command_response(cmds_len_before, 200) };
        drop(requester);
        response
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
    /// Checks all root hub ports for connected ports and adds them to the port connection queue
    pub fn prob(&self) {
        let regs = unsafe { self.regs.as_mut_unchecked() };
        let caps = unsafe { regs.captabilities() };
        let op_regs = unsafe { regs.operational_regs() };
        // Resettng all the root hub ports
        // TODO: detect connections
        for i in 0..caps.max_ports() {
            let port_regs = op_regs.port_registers(i);
            let port_sc = read_ref!(port_regs.port_sc);

            if port_sc.ccs() && port_sc.csc() {
                self.manager_queue
                    .add_port_connection_event(i, !port_sc.ccs());
            }
        }
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
        let trb = trb::TRB::new(
            trb::TRBCommand::default().with_trb_type(TRB_TYPE_ENABLE_SLOT_CMD),
            0,
            0,
        );
        let response = self.manager_queue.send_command(trb);
        crate::serial!("XHCI responded with {response:#x?}\n");
        true
    }
}
