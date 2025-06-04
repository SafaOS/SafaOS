use lazy_static::lazy_static;

use crate::{
    arch::paging::current_higher_root_table,
    info,
    memory::{
        frame_allocator::Frame,
        paging::{EntryFlags, MapToError, Page},
    },
    VirtAddr,
};

use super::paging::PageTable;

lazy_static! {
    static ref GICD_BASE: VirtAddr = super::cpu::GIC.0.into_virt();
    static ref GICC_BASE: VirtAddr = super::cpu::GIC.1.into_virt();
}

#[inline(always)]
fn gicd_ctlr() -> *mut u32 {
    GICD_BASE.into_ptr::<u32>()
}

#[inline(always)]
fn gicd_isenabler() -> *mut u32 {
    (*GICD_BASE + 0x100).into_ptr::<u32>()
}

#[inline(always)]
fn gicc_ctlr() -> *mut u32 {
    GICC_BASE.into_ptr::<u32>()
}

#[inline(always)]
fn gicc_pmr() -> *mut u32 {
    (*GICC_BASE + 0x4).into_ptr::<u32>()
}

#[inline(always)]
fn gicc_bpr() -> *mut u32 {
    (*GICC_BASE + 0x8).into_ptr::<u32>()
}

#[inline(always)]
fn gicd_icpendr() -> *mut u32 {
    (*GICD_BASE + 0x0280).into_ptr::<u32>()
}

unsafe fn map_gic(dest: &mut PageTable) -> Result<(), MapToError> {
    let flags = EntryFlags::WRITE;
    dest.map_to(
        Page::containing_address(*GICC_BASE),
        Frame::containing_address(GICC_BASE.into_phys()),
        flags,
    )?;
    dest.map_to(
        Page::containing_address(*GICD_BASE),
        Frame::containing_address(GICD_BASE.into_phys()),
        flags,
    )?;
    Ok(())
}
pub fn init_gic() {
    unsafe {
        map_gic(&mut *current_higher_root_table()).expect("failed to map gic");
    }
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

#[inline(always)]
fn gicc_iar() -> *mut u32 {
    (*GICC_BASE + 0xC).into_ptr::<u32>()
}

/// Gets the interrupt ID of the current ingoing interrupt
#[inline(always)]
pub fn get_int_id() -> u32 {
    unsafe { (*gicc_iar()) & 0xFFFFFF }
}

fn gicd_ispendr0() -> *mut u32 {
    (*GICD_BASE + 0x200).into_ptr::<u32>()
}

pub fn set_pending(interrupt: u32) {
    assert!(interrupt <= 32);
    let ispender = gicd_ispendr0();
    unsafe {
        core::ptr::write_volatile(ispender, 1 << interrupt);
    }
}
