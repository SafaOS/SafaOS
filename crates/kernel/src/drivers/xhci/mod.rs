use regs::{CapsReg, OperationalRegs, RuntimeRegs, USBCmd, USBSts, XHCIIman};
use rings::XHCICommandRing;
use utils::{allocate_buffers_frame, read_ref, write_ref};

use crate::{
    arch::paging::current_higher_root_table,
    debug,
    drivers::pci::PCICommandReg,
    memory::{
        align_up,
        frame_allocator::{self, Frame},
        paging::{EntryFlags, PAGE_SIZE},
    },
    time, PhysAddr, VirtAddr,
};

use super::pci::PCIDevice;
mod regs;
mod rings;
mod utils;

/// The maximum number of TRBs a CommandRing can hold
const MAX_TRB_COUNT: usize = 256;

#[derive(Debug)]
pub struct XHCI<'s> {
    virt_base_addr: VirtAddr,
    // TODO: free the frames when this goes out of scope? except that currently it never does
    /// used to store the scratchpad_buffers pointers and the dcbaa (scratchpad_buffers, dcbaa)
    buffers_frame: Frame,
    scratchpad_buffers: Option<&'s mut [Frame]>,
    dcbaa: &'s mut [PhysAddr],

    command_ring: XHCICommandRing<'s>,
}

impl<'s> XHCI<'s> {
    fn captabilities<'a>(&self) -> &'a CapsReg {
        unsafe { &*self.virt_base_addr.into_ptr::<CapsReg>() }
    }

    fn captabilities_mut<'a>(&mut self) -> &'a mut CapsReg {
        unsafe { &mut *self.virt_base_addr.into_ptr::<CapsReg>() }
    }

    fn operational_regs<'a>(&mut self) -> &'a mut OperationalRegs {
        self.captabilities_mut().operational_regs_mut()
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
        let interrupt_reg = &mut runtime_regs.interrupt_registers[interrupter as usize];
        // Similariy we clear the iman interrupt pending bit by writing 1 to it
        let iman = interrupt_reg.iman | XHCIIman::INTERRUPT_PENDING;
        write_ref!(interrupt_reg.iman, iman);
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
        // reconfigure the controller
        self.reconfigure();
        debug!(XHCI, "XHCI Reset\n{}", regs);
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
        let runtime_regs = self.runtime_regs();
        let interrupt_reg = &mut runtime_regs.interrupt_registers[0];
        // Enable interrupts
        let iman = interrupt_reg.iman | XHCIIman::INTERRUPT_ENABLE;
        write_ref!(interrupt_reg.iman, iman);
        // Clear any pending interrupts
        self.acknowledge_irq(0);
    }
}

impl<'s> PCIDevice for XHCI<'s> {
    fn class() -> (u8, u8, u8) {
        (0xc, 0x3, 0x30)
    }

    fn create(mut header: super::pci::PCIHeader) -> Self {
        let header = header.unwrap_general();
        write_ref!(
            header.common.command,
            PCICommandReg::BUS_MASTER | PCICommandReg::MEM_SPACE
        );

        let (base_addr, size) = header.get_bars()[0];
        let virt_base_addr = base_addr.into_virt();

        unsafe {
            let page_num = size.div_ceil(PAGE_SIZE);
            current_higher_root_table()
                .map_contiguous_pages(
                    virt_base_addr,
                    base_addr,
                    page_num,
                    EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE,
                )
                .expect("failed to map the XHCI");
        }

        let mut results = Self {
            virt_base_addr,
            buffers_frame: frame_allocator::allocate_frame()
                .expect("XHCI: failed to allocate memory"),
            scratchpad_buffers: None,
            dcbaa: &mut [],
            command_ring: XHCICommandRing::create(MAX_TRB_COUNT),
        };
        debug!(
            XHCI,
            "Mapped\n{}\n{}",
            results.captabilities(),
            results.operational_regs()
        );
        results.reset();
        results
    }

    fn name(&self) -> &'static str {
        "XHCI"
    }
}
