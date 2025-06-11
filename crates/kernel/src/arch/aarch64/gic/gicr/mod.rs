use bitfield_struct::bitfield;

use crate::{arch::aarch64::gic::gicr::lpis::LPI_MANAGER, info};

use super::{GICR_BASE, SGI_BASE};

pub mod lpis;

/// Wakes up and initializes the GICR
pub fn init() {
    GICRWaker::write(GICRWaker::new().with_processor_sleep(false));
    assert!(!GICRWaker::read_vol().processor_sleep());
    // Polls until it wakes up
    while GICRWaker::read_vol().children_asleep() {
        core::hint::spin_loop();
    }

    let gicr_typer = GICRTyper::get();
    info!(
        "woke up the GICR, processor num: {}, is the last GICR: {}, supports direct lpis: {}",
        gicr_typer.processor_num(),
        gicr_typer.last(),
        gicr_typer.direct_lpi()
    );

    unsafe {
        LPI_MANAGER.lock().init();
        GICRCtlr::read().with_enable_lpis(true).write();
        info!("initialized LPI configuration table and pending table");
    }
}

#[bitfield(u32)]
pub struct GICRCtlr {
    /// In implementations where affinity routing is enabled for the Security state:
    ///
    /// 0b0 LPI support is disabled. Any doorbell interrupt generated as a result of a write to a
    /// virtual LPI register must be discarded, and any ITS translation requests or commands
    /// involving LPIs in this Redistributor are ignored.
    ///
    /// 0b1 LPI support is enabled.
    enable_lpis: bool,
    /// Clear Enable Supported.
    ///
    /// This bit is read-only.
    ///
    /// 0b0 The IRI does not indicate whether GICR_CTLR.EnableLPIs is RES1 once set.
    ///
    /// 0b1 GICR_CTLR.EnableLPIs is not RES1 once set.
    ///
    /// Implementing GICR_CTLR.EnableLPIs as programmable and not reporting GICR_CLTR.CES ==
    /// 1 is deprecated.
    ///
    /// Implementing GICR_CTLR.EnableLPIs as RES1 once set is deprecated.
    ///
    /// When GICR_CLTR.CES == 0, software cannot assume that GICR_CTLR.EnableLPIs is
    /// programmable without observing the bit being cleared.
    ces: bool,
    /// LPI invalidate registers supported.
    ///
    /// This bit is read-only.
    ///
    /// 0b0 This bit does not indicate whether the GICR_INVLPIR, GICR_INVALLR and
    ///
    /// GICR_SYNCR are implemented or not.
    ///
    /// 0b1 GICR_INVLPIR, GICR_INVALLR and GICR_SYNCR are implemented.
    /// If GICR_TYPER.DirectLPI is 1 or GICR_TYPER.RVPEI is 1, GICR_INVLPIR,
    /// GICR_INVALLR, and GICR_SYNCR are always implemented
    ir: bool,
    /// Register Write Pending. This bit indicates whether a register write for the current Security state is
    /// in progress or not.
    rwp: bool,
    #[bits(20)]
    __: (),
    /// Disable Processor selection for Group 0 interrupts. When GICR_TYPER.DPGS == 1:
    ///
    /// 0b0 A Group 0 SPI configured to use the 1 of N distribution model can select this PE, if the
    /// PE is not asleep and if Group 0 interrupts are enabled.
    ///
    /// 0b1 A Group 0 SPI configured to use the 1 of N distribution model cannot select this PE.
    dpg0: bool,
    /// Disable Processor selection for Group 1 Non-secure interrupts. When GICR_TYPER.DPGS == 1:
    ///
    /// 0b0 A Group 1 Non-secure SPI configured to use the 1 of N distribution model can select
    ///
    /// this PE, if the PE is not asleep and if Non-secure Group 1 interrupts are enabled.
    /// 0b1 A Group 1 Non-secure SPI configured to use the 1 of N distribution model cannot select
    /// this PE.
    dpg1ns: bool,
    /// Disable Processor selection for Group 1 Secure interrupts. When GICR_TYPER.DPGS == 1:
    ///
    /// 0b0 A Group 1 Secure SPI configured to use the 1 of N distribution model can select this
    /// PE, if the PE is not asleep and if Secure Group 1 interrupts are enabled.
    ///
    /// 0b1 A Group 1 Secure SPI configured to use the 1 of N distribution model cannot select this
    /// PE.
    dpg1s: bool,
    #[bits(4)]
    __: (),
    /// Upstream Write Pending. Read-only. Indicates whether all upstream writes have been
    /// communicated to the Distributor
    uwp: bool,
}

impl GICRCtlr {
    pub fn get_ptr() -> *mut Self {
        GICR_BASE.into_ptr()
    }

    pub fn read() -> Self {
        unsafe { Self::get_ptr().read() }
    }

    pub unsafe fn write(self) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(), self);
        }
    }
}

// TODO: docs?
#[bitfield(u64)]
pub struct GICRTyper {
    plpis: bool,
    vlpis: bool,
    dirty: bool,
    direct_lpi: bool,
    last: bool,
    dpgs: bool,
    mpam: bool,
    rvpeid: bool,
    processor_num: u16,
    #[bits(2)]
    common_lpi_af: u8,
    vsgi: bool,
    #[bits(5)]
    ppi_num: u8,
    af_value: u32,
}

impl GICRTyper {
    fn get() -> Self {
        unsafe { Self::from_bits(core::ptr::read((*GICR_BASE + 0x8).into_ptr::<u64>())) }
    }
}

#[bitfield(u32)]
pub struct GICRWaker {
    /// IMPLEMENTITION DEFINED
    #[bits(1)]
    __: (),

    processor_sleep: bool,
    children_asleep: bool,

    #[bits(28)]
    __: (),

    /// IMPLEMENTITION DEFINED
    #[bits(1)]
    __: (),
}

impl GICRWaker {
    pub fn get_ptr() -> *mut Self {
        (*GICR_BASE + 0x14).into_ptr::<_>()
    }
    /// Performs a volitate write to the GICR_WAKER register
    pub fn write(src: Self) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(), src);
        }
    }
    /// Performs a volitate read to retrieve self
    pub fn read_vol() -> Self {
        unsafe { core::ptr::read_volatile(Self::get_ptr()) }
    }
}

/// Pointer to the GICR_ISENABLER<0> Register
#[inline(always)]
pub fn isenabler() -> *mut u32 {
    (*SGI_BASE + 0x100).into_ptr::<u32>()
}

/// Pointer to the GICR_ICPENDR<0> Register
#[inline(always)]
pub fn icpendr0() -> *mut u32 {
    (*SGI_BASE + 0x0280).into_ptr::<u32>()
}

/// Pointer to the GICR_IGROUP<0> Register
#[inline(always)]
pub fn igroup0() -> *mut u32 {
    (*SGI_BASE + 0x0080).into_ptr::<u32>()
}

/// Pointer to the GICR_ISPENDR<0> Register
#[inline(always)]
pub fn ispendr0() -> *mut u32 {
    (*SGI_BASE + 0x200).into_ptr::<u32>()
}
