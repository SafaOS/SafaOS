use core::arch::asm;

use crate::{
    VirtAddr,
    arch::paging::{CURRENT_HIGHER_HALF_TABLE, set_current_higher_page_table_phys},
    percpu::CpuLocal,
};

unsafe fn set_tpidr(value: VirtAddr) {
    crate::serial!("tpidr_el1 set to: {value:#x}\n");
    unsafe {
        asm!("msr tpidr_el1, {}", in(reg) value.into_raw(), options(nomem, nostack));
    }
}

#[inline(always)]
pub fn current_local_ptr() -> *mut CpuLocal {
    let ptr: *mut CpuLocal;
    unsafe { asm!("mrs {}, tpidr_el1", out(reg) ptr, options(nostack, nomem)) }
    ptr
}

/// Sets up MP related stuff.
pub fn setup_cpu_mp(local: &'static CpuLocal) {
    unsafe { set_tpidr(VirtAddr::from_ptr(local)) };
}

/// Given a base CPU Local Storage reference, initialize the CPU with it.
pub fn init_cpu_with(local: &'static CpuLocal) {
    super::setup_cpu_basics();
    let phys_addr = unsafe { *CURRENT_HIGHER_HALF_TABLE.get() };
    unsafe { set_current_higher_page_table_phys(phys_addr) };

    setup_cpu_mp(local);
    super::setup_cpu_pherphials();
}
