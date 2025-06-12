use super::{
    interrupts::IRQInfo,
    utils::{read_ref, write_ref},
};
use regs::{CapsReg, XHCIDoorbellManager};
use rings::{TRBCommand, XHCICommandRing, XHCIEventRing};

use crate::{
    arch::paging::current_higher_root_table,
    debug,
    drivers::{
        interrupts::{self, IntTrigger, InterruptReceiver},
        pci::PCICommandReg,
        xhci::{regs::XHCIRegisters, rings::TRB_TYPE_ENABLE_SLOT_CMD},
    },
    memory::paging::{EntryFlags, PAGE_SIZE},
    utils::locks::Mutex,
};

use super::pci::PCIDevice;
mod regs;
mod rings;
mod utils;

/// The maximum number of TRBs a CommandRing can hold
const MAX_TRB_COUNT: usize = 256;

// TODO: maybe stack interrupt stuff together in one struct behind a Mutex?
/// The main XHCI driver Instance
#[derive(Debug)]
pub struct XHCI<'s> {
    // TODO: maybe use a UnsafeCell here?
    /// be careful using the registers, should only be used while interrupts are disabled
    regs: Mutex<XHCIRegisters<'s>>,
    /// Not accessed by interrupts
    command_ring: Mutex<XHCICommandRing<'s>>,
    /// Only accessed by interrupts
    event_ring: Mutex<XHCIEventRing<'s>>,
    /// Not accessed by interrupts
    doorbell_manager: Mutex<XHCIDoorbellManager<'s>>,
    irq_info: IRQInfo,
}

impl<'s> InterruptReceiver for XHCI<'s> {
    fn handle_interrupt(&self) {
        let events = self.event_ring.lock().dequeue_events();
        crate::serial!("{events:#x?}\n");
        self.regs.lock().acknowledge_irq(0);
    }
}

impl<'s> XHCI<'s> {}

unsafe impl<'s> Send for XHCI<'s> {}
unsafe impl<'s> Sync for XHCI<'s> {}

impl<'s> PCIDevice for XHCI<'s> {
    fn class() -> (u8, u8, u8) {
        (0xc, 0x3, 0x30)
    }

    fn create(mut info: super::pci::PCIDeviceInfo) -> Self {
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

        let caps_ptr = virt_base_addr.into_ptr::<CapsReg>();
        let caps_regs = unsafe { &mut *caps_ptr };

        let runtime_regs = caps_regs.runtime_regs_mut();
        let interrupter = runtime_regs.interrupter_mut(0);

        let command_ring = XHCICommandRing::create(MAX_TRB_COUNT);
        let mut event_ring = XHCIEventRing::create(MAX_TRB_COUNT, interrupter);

        let mut xhci_registers = unsafe { XHCIRegisters::new(caps_regs) };
        unsafe {
            xhci_registers.reconfigure(&mut event_ring, &command_ring);
        }

        let doorbell_manager =
            XHCIDoorbellManager::new(caps_regs.doorbells_base(), caps_regs.max_device_slots());

        // FIXME: switch to MSI if not available
        let irq_info = info
            .get_msix_cap()
            .map(|msix| msix.into_irq_info())
            .unwrap();

        let mut this = XHCI {
            command_ring: Mutex::new(command_ring),
            event_ring: Mutex::new(event_ring),
            doorbell_manager: Mutex::new(doorbell_manager),
            regs: Mutex::new(xhci_registers),
            irq_info,
        };
        debug!(
            XHCI,
            "Created\n{}\n{}",
            this.regs.get_mut().captabilities(),
            this.regs.get_mut().operational_regs()
        );
        this
    }

    fn start(&'static self) -> bool {
        let irq_info = self.irq_info.clone();
        interrupts::register_irq(irq_info, IntTrigger::Edge, self);

        let op_regs = self.regs.lock().operational_regs();
        let usbsts_before = read_ref!(op_regs.usbstatus);
        let usbcmd_before = read_ref!(op_regs.usbcmd);
        unsafe {
            self.regs.lock().start();
        }
        let usbsts_after = read_ref!(op_regs.usbstatus);
        let usbcmd_after = read_ref!(op_regs.usbcmd);
        debug!(
            XHCI,
            "Started, usbsts before {:?} => usbsts after {:?}, usbcmd before {:?} => usbcmd after {:?}", usbsts_before, usbsts_after, usbcmd_before, usbcmd_after
        );

        let trb = rings::TRB::new(
            TRBCommand::default().with_trb_type(TRB_TYPE_ENABLE_SLOT_CMD),
            0,
            0,
        );
        self.command_ring.lock().enqueue(trb);
        self.doorbell_manager.lock().ring_command_doorbell();
        true
    }
}
