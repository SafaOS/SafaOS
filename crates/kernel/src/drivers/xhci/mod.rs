use super::{
    interrupts::IRQInfo,
    utils::{read_ref, write_ref},
};
use regs::{CapsReg, OperationalRegs, RuntimeRegs, USBCmd, USBSts, XHCIDoorbellManager, XHCIIman};
use rings::{TRBCommand, XHCICommandRing, XHCIEventRing};
use utils::allocate_buffers_frame;

use crate::{
    arch::paging::current_higher_root_table,
    debug,
    drivers::{
        interrupts::{self, IntTrigger, InterruptReceiver},
        pci::PCICommandReg,
    },
    memory::{
        align_up,
        frame_allocator::{self, Frame},
        paging::{EntryFlags, PAGE_SIZE},
    },
    time,
    utils::locks::Mutex,
    PhysAddr,
};

use super::pci::PCIDevice;
mod regs;
mod rings;
mod utils;

/// The maximum number of TRBs a CommandRing can hold
const MAX_TRB_COUNT: usize = 256;

#[derive(Debug)]
pub struct XHCI<'s> {
    caps_regs: *mut CapsReg,
    // TODO: free the frames when this goes out of scope? except that currently it never does
    /// used to store the scratchpad_buffers pointers and the dcbaa (scratchpad_buffers, dcbaa)
    buffers_frame: Frame,
    scratchpad_buffers: Option<&'s mut [Frame]>,
    dcbaa: &'s mut [PhysAddr],

    command_ring: XHCICommandRing<'s>,
    event_ring: XHCIEventRing<'s>,
    doorbell_manager: XHCIDoorbellManager<'s>,
    irq_info: IRQInfo,
}

impl<'s> InterruptReceiver for Mutex<XHCI<'s>> {
    fn handle_interrupt(&self) {
        crate::serial!("ENTERED HANDLER\n");
        todo!("{}", self.lock().operational_regs())
    }
}

impl<'s> XHCI<'s> {
    fn captabilities<'a>(&self) -> &'a CapsReg {
        unsafe { &*self.caps_regs }
    }

    fn captabilities_mut<'a>(&mut self) -> &'a mut CapsReg {
        unsafe { &mut *self.caps_regs }
    }

    fn operational_regs<'a>(&mut self) -> &'a mut OperationalRegs {
        let caps = self.captabilities_mut();
        caps.operational_regs_mut()
    }

    fn runtime_regs<'a>(&mut self) -> &'a mut RuntimeRegs {
        self.captabilities_mut().runtime_regs_mut()
    }

    /// Clear any incoming interrupts for the interrupter
    pub fn acknowledge_irq(&mut self, interrupter: u8) {
        let op_regs = self.operational_regs();
        // Write the USBSts::EINT bit to clear it, it is RW1C meaning write 1 to clear
        write_ref!(op_regs.usbstatus, USBSts::EINT);

        let runtime_regs = self.runtime_regs();
        let interrupt_reg = runtime_regs.interrupter_mut(interrupter as usize);
        // Similariy we clear the iman interrupt pending bit by writing 1 to it
        let iman = interrupt_reg.iman | XHCIIman::INTERRUPT_PENDING;
        write_ref!(interrupt_reg.iman, iman);
    }

    fn start(&mut self) {
        let regs = self.operational_regs();
        write_ref!(
            regs.usbcmd,
            regs.usbcmd | USBCmd::RUN | USBCmd::INTERRUPT_ENABLE
        );

        let timeout = 1000;
        let time = time!();

        while read_ref!(regs.usbstatus).contains(USBSts::HCHALTED) {
            let now = time!();
            if now >= time + timeout {
                panic!(
                    "timeout after {}ms while resetting the XHCI, HCHALTED did not clear: {:?}",
                    now,
                    read_ref!(regs.usbstatus)
                )
            }
            core::hint::spin_loop();
        }

        assert!(!read_ref!(regs.usbstatus).contains(USBSts::NOT_READY));
    }

    #[allow(unused_unsafe)]
    /// Resets the XHCI controller
    fn reset(&mut self) {
        let regs = self.operational_regs();

        write_ref!(regs.usbcmd, regs.usbcmd & !USBCmd::RUN);

        let timeout = 200;
        let time = time!();

        while !read_ref!(regs.usbstatus).contains(USBSts::HCHALTED) {
            let now = time!();
            if now >= time + timeout {
                panic!(
                    "timeout after {}ms while resetting the XHCI, HCHALTED did not set: {:?}",
                    now,
                    read_ref!(regs.usbstatus)
                )
            }
            core::hint::spin_loop();
        }

        // reset the controller
        write_ref!(regs.usbcmd, read_ref!(regs.usbcmd) | USBCmd::HCRESET);

        let timeout = 1000;
        let time = time!();

        while read_ref!(regs.usbcmd).contains(USBCmd::HCRESET)
            || read_ref!(regs.usbstatus).contains(USBSts::NOT_READY)
        {
            let now = time!();
            if now >= time + timeout {
                panic!(
                    "timeout after {}ms while resetting controller, controller was never ready: {:?}",
                    now - time,
                    read_ref!(regs.usbcmd),
                )
            }
            core::hint::spin_loop();
        }
        // asserts the controller was reset
        assert_eq!(regs.usbcmd, USBCmd::empty());
        assert_eq!(regs.dnctrl, 0);
        assert_eq!(regs.crcr, 0);
        assert_eq!(regs.dcbaap, PhysAddr::null());
        assert_eq!(regs.config, 0);
        write_ref!(regs.config, self.captabilities().max_device_slots() as u32);
        // reconfigure the controller
        self.reconfigure();
        debug!(
            XHCI,
            "XHCI Reset\n{}\n{:#x?}",
            regs,
            self.runtime_regs().interrupter(0)
        );
    }

    fn reconfigure(&mut self) {
        let op_regs = self.operational_regs();
        // Enable device notifications
        write_ref!(op_regs.dnctrl, 0xFFFF);
        self.configure_dcbaa();
        self.configure_crcr();

        self.configure_runtime();
    }

    fn configure_crcr(&mut self) {
        let op_regs = self.operational_regs();
        write_ref!(
            op_regs.crcr,
            *self.command_ring.base_phys_addr() | self.command_ring.current_ring_cycle() as usize
        );
    }

    fn configure_dcbaa(&mut self) {
        let caps = self.captabilities();
        let op_regs = self.operational_regs();

        // Allocates and sets the dcbaa
        assert!(caps.max_device_slots() * size_of::<PhysAddr>() <= PAGE_SIZE);

        let (dcbaa_slice, dcbaa_phys_addr) =
            allocate_buffers_frame::<PhysAddr>(self.buffers_frame, 0, caps.max_device_slots());

        // Allocates the scratchpad buffers array if neccassary
        if caps.max_scratchpad_buffers() > 0 {
            // uses the same frame to store the scratchpad_buffers pointers that we used to store dcbaa entries
            // it is safe to do so as the max number of dcbaa entries is 255,
            // and the max numbers of scratchpad_buffers is 15, (255 + 15) * 8 is very much less then the maximum amount of bytes a frame (page) can hold (4096)
            // DCBAA entries must be 64 byte aligned
            let (scratchpad_buffers, scratchpad_buffers_addr) = allocate_buffers_frame::<Frame>(
                self.buffers_frame,
                align_up((dcbaa_phys_addr + dcbaa_slice.len()).into_raw(), 64),
                caps.max_scratchpad_buffers(),
            );

            for phys_addr in scratchpad_buffers.iter_mut() {
                *phys_addr = frame_allocator::allocate_frame()
                    .expect("XHCI: failed to allocate a page for a scratchpad buffer");
            }
            self.scratchpad_buffers = Some(scratchpad_buffers);
            // DCBAA[0] is used to store the address of the scratchpad_buffers
            self.dcbaa[0] = scratchpad_buffers_addr;
        }

        self.dcbaa = dcbaa_slice;
        write_ref!(op_regs.dcbaap, dcbaa_phys_addr);
    }

    fn configure_runtime(&mut self) {
        self.event_ring.reset();
        let runtime_regs = self.runtime_regs();
        let interrupt_reg = runtime_regs.interrupter_mut(0);
        // Enable interrupts
        write_ref!(interrupt_reg.iman, XHCIIman::INTERRUPT_ENABLE);

        // Clear any pending interrupts
        self.acknowledge_irq(0);
    }
}

unsafe impl<'s> Send for XHCI<'s> {}
unsafe impl<'s> Sync for XHCI<'s> {}

impl<'s> PCIDevice for Mutex<XHCI<'s>> {
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
        let event_ring = XHCIEventRing::create(MAX_TRB_COUNT, interrupter);

        let doorbell_manager =
            XHCIDoorbellManager::new(caps_regs.doorbells_base(), caps_regs.max_device_slots());

        // FIXME: switch to MSI if not available
        let irq_info = info
            .get_msix_cap()
            .map(|msix| msix.into_irq_info())
            .unwrap();

        let mut results = Mutex::new(XHCI {
            buffers_frame: frame_allocator::allocate_frame()
                .expect("XHCI: failed to allocate memory"),
            scratchpad_buffers: None,
            dcbaa: &mut [],
            command_ring,
            event_ring,
            caps_regs,
            doorbell_manager,
            irq_info,
        });
        let this = results.get_mut();
        debug!(
            XHCI,
            "Mapped\n{}\n{}",
            this.captabilities(),
            this.operational_regs()
        );
        this.reset();
        results
    }

    fn start(&'static self) -> bool {
        unsafe {
            crate::arch::disable_interrupts();
        }
        let irq_info = self.lock().irq_info.clone();
        interrupts::register_irq(irq_info, IntTrigger::Edge, self);

        let op_regs = self.lock().operational_regs();
        let usbsts_before = read_ref!(op_regs.usbstatus);
        let usbcmd_before = read_ref!(op_regs.usbcmd);
        self.lock().start();
        let usbsts_after = read_ref!(op_regs.usbstatus);
        let usbcmd_after = read_ref!(op_regs.usbcmd);
        debug!(
            XHCI,
            "Started, usbsts before {:?} => usbsts after {:?}, usbcmd before {:?} => usbcmd after {:?}", usbsts_before, usbsts_after, usbcmd_before, usbcmd_after
        );

        let trb = rings::TRB::new(TRBCommand::default().with_trb_type(9), 0, 0);
        self.lock().command_ring.enqueue(trb);
        self.lock().doorbell_manager.ring_command_doorbell();
        unsafe {
            crate::arch::enable_interrupts();
        }
        true
    }
}
