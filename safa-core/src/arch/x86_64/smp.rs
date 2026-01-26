use crate::{
    VirtAddr,
    arch::{
        self,
        paging::{CURRENT_RING0_PAGE_TABLE, set_current_page_table_phys},
        x86_64::{gdt::init_gdt, interrupts::init_idt, registers::wrmsr},
    },
    debug, info,
    percpu::CpuLocal,
};

pub unsafe fn set_gs(value: VirtAddr) {
    unsafe {
        wrmsr(0xC0000101, value.into_raw() as u64);
        wrmsr(0xC0000102, value.into_raw() as u64);
        core::arch::asm!("swapgs");
    }
}

#[inline(always)]
pub fn current_local_ptr() -> *mut CpuLocal {
    let ptr: *mut CpuLocal;
    unsafe { core::arch::asm!("mov {}, gs:0", out(reg) ptr, options(nostack, preserves_flags)) }
    ptr
}

/// Given a base CPU Local Storage reference, initialize the CPU with it.
pub fn init_cpu_with(local: &'static CpuLocal) {
    let phys_addr = unsafe { *CURRENT_RING0_PAGE_TABLE.get() };
    unsafe { set_current_page_table_phys(phys_addr) };

    init_gdt(local);
    debug!("{}: GDT init... Ok", local.cpu_id);
    init_idt();
    info!("{}: IDT init... Ok", local.cpu_id);
    info!(
        "{}: APIC ID {} is entering phase 2 of initialization",
        local.cpu_id, local.cpu_arch_id
    );
    arch::x86_64::setup_cpu_generic2();
}
