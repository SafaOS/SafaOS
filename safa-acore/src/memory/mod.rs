use crate::{
    bootloader::HHDM,
    misc::{PhysAddr, VirtAddr},
};

pub mod frame_allocator;

pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(HHDM.raw() | phys.raw())
}

/// Initializes the physical memory manager.
pub unsafe fn init_pmm() {
    frame_allocator::init();
}
