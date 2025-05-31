use core::fmt::Display;

use crate::{
    arch::paging::current_higher_root_table,
    debug,
    memory::paging::{EntryFlags, PAGE_SIZE},
    VirtAddr,
};

use super::pci::PCIDevice;

#[repr(C)]
struct CapsReg {
    reg_length: u8,
    _reserved0: u8,
    version_number: u8,
    hcsparams_1: u32,
    hcsparams_2: u32,
    hcsparams_3: u32,
    hccparams_1: u32,
    dorbell_off: u32,
    runtime_off: u32,
    hccparams_2: u32,
}

impl CapsReg {
    const fn max_device_slots(&self) -> u8 {
        self.hcsparams_1 as u8
    }
    const fn max_interrupts(&self) -> u8 {
        (self.hcsparams_1 >> 8) as u8
    }
    const fn max_ports(&self) -> u8 {
        (self.hcsparams_1 >> 24) as u8
    }
    const fn interrupt_schd_t(&self) -> u8 {
        (self.hcsparams_2 as u8) & 0xF
    }
    const fn erst_max(&self) -> u8 {
        ((self.hcsparams_2 >> 4) as u8) & 0xF
    }
    const fn max_scratchpad_buffers(&self) -> u8 {
        ((self.hcsparams_2 >> 21) as u8) & 0x1F
    }
    const fn addressing_64bits(&self) -> bool {
        (self.hccparams_1 & 0x1) != 0
    }
    const fn bandwidth_negotiation(&self) -> bool {
        ((self.hccparams_1 >> 1) & 0x1) != 0
    }
    const fn context_sz_64bytes(&self) -> bool {
        ((self.hccparams_1 >> 2) & 0x1) != 0
    }
    const fn port_power_ctrl(&self) -> bool {
        ((self.hccparams_1 >> 3) & 0x1) != 0
    }
    const fn port_indicator_ctrl(&self) -> bool {
        ((self.hccparams_1 >> 4) & 0x1) != 0
    }
    const fn light_reset_support(&self) -> bool {
        ((self.hccparams_1 >> 5) & 0x1) != 0
    }
}

impl Display for CapsReg {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "XHCI Captabilites Register @{:?}:", VirtAddr::from_ptr(self as *const _))?;
        writeln!(f, "\tLength                            : {:#x}", self.reg_length)?;
        writeln!(f, "\tMax Device Slots                  : {}", self.max_device_slots())?;
        writeln!(f, "\tMax Interrupts                    : {}", self.max_interrupts())?;
        writeln!(f, "\tMax Ports                         : {}", self.max_ports())?;
        writeln!(f, "\tIST                               : {}", self.interrupt_schd_t())?;
        writeln!(f, "\tERST Max Size                     : {}", self.erst_max())?;
        writeln!(f, "\tScratchpad Buffers                : {}", self.max_scratchpad_buffers())?;
        writeln!(f, "\t64-bit Addressing                 : {}" ,self.addressing_64bits())?;
        writeln!(f, "\tBandwidth Negotiation Implemented : {}", self.bandwidth_negotiation())?;
        writeln!(f, "\t64-byte Context Size              : {}", self.context_sz_64bytes())?;
        writeln!(f, "\tPort Power Control                : {}", self.port_power_ctrl())?;
        writeln!(f, "\tPort Indicators Control           : {}", self.port_indicator_ctrl())?;
        write!(f,   "\tLight Reset Available             : {}", self.light_reset_support())
    }
}

#[derive(Debug)]
pub struct XHCI {
    virt_base_addr: VirtAddr,
}

impl XHCI {
    fn captabilities(&self) -> &CapsReg {
        unsafe { &*self.virt_base_addr.into_ptr::<CapsReg>() }
    }
}

impl PCIDevice for XHCI {
    fn class() -> (u8, u8, u8) {
        (0xc, 0x3, 0x30)
    }

    fn create(header: super::pci::PCIHeader) -> Self {
        let header = header.unwrap_general();

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

        let results = Self { virt_base_addr };
        debug!(XHCI, "Mapped\n{}", results.captabilities());
        results
    }

    fn name(&self) -> &'static str {
        "XHCI"
    }
}
impl XHCI {}
