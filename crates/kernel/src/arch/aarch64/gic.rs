use lazy_static::lazy_static;

use crate::info;

lazy_static! {
    // FIXME: only works for qemu virt
    static ref GICD_BASE: usize = *crate::limine::HDDM + 0x08000000;
    static ref GICC_BASE: usize = *crate::limine::HDDM + 0x08010000;
}

#[inline(always)]
fn gicd_ctlr() -> *mut u32 {
    *GICD_BASE as *mut u32
}

#[inline(always)]
fn gicd_isenabler() -> *mut u32 {
    (*GICD_BASE + 0x100) as *mut u32
}

#[inline(always)]
fn gicc_ctlr() -> *mut u32 {
    *GICC_BASE as *mut u32
}

#[inline(always)]
fn gicc_pmr() -> *mut u32 {
    (*GICC_BASE + 0x4) as *mut u32
}

#[inline(always)]
fn gicc_bpr() -> *mut u32 {
    (*GICC_BASE + 0x8) as *mut u32
}

#[inline(always)]
fn gicd_icpendr() -> *mut u32 {
    (*GICD_BASE + 0x0280) as *mut u32
}

pub fn init_gic() {
    let gicd_ctlr = gicd_ctlr();
    let gicc_ctlr = gicc_ctlr();
    info!(
        "initializing GIC GICD: {:?}, GICC: {:?}",
        gicd_ctlr, gicc_ctlr
    );

    unsafe {
        core::ptr::write_volatile(gicd_ctlr, 1);
        core::ptr::write_volatile(gicc_ctlr, 1);

        let gicc_pmr = gicc_pmr();
        let gicc_bpr = gicc_bpr();

        core::ptr::write_volatile(gicc_pmr, 0xff);
        // No groups
        core::ptr::write_volatile(gicc_bpr, 0);
    }
}

/// Enables GIC interrupt
pub fn enable(interrupt: u32) {
    assert!(interrupt <= 32);
    unsafe {
        let reg = gicd_isenabler();
        core::ptr::write_volatile(reg, 1 << interrupt);
    }
}

#[inline(always)]
/// Clears pending interrupt
pub fn clear_pending(interrupt: u32) {
    assert!(interrupt <= 32);
    unsafe {
        let gicd_icpendr = gicd_icpendr();
        core::ptr::write_volatile(gicd_icpendr, 1 << interrupt);
    }
}
