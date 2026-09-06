use crate::{
    bootloader::HHDM,
    misc::{PhysAddr, VirtAddr},
};

pub mod pmm;
pub(super) mod region_list_allocator;
pub mod slab_allocator;
pub mod vmm;

#[inline(always)]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(HHDM.raw() | phys.raw())
}

#[inline(always)]
pub fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    PhysAddr::new(virt.raw() - HHDM.raw())
}

/// Initializes the physical memory manager.
pub unsafe fn init_pmm() {
    pmm::init();
}
