use bitfield_struct::bitfield;

use crate::info;

use super::{GICR_BASE, SGI_BASE};

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
