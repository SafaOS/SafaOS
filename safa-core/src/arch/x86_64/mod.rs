mod acpi;
mod gdt;
mod pit;
mod tsc;

pub(super) mod interrupts;
pub(super) mod io;
pub mod paging;
pub(super) mod pci;
pub(super) mod power;
pub(crate) mod registers;
pub(super) mod serial;
pub mod smp;
mod syscalls;
#[cfg(test)]
mod tests;
pub(super) mod threading;
pub(super) mod tlb;
pub(super) mod utils;

use core::{arch::asm, sync::atomic::Ordering};
use interrupts::{apic, init_idt};
use serial::init_serial;

use crate::{
    arch::{
        registers::ArchCpuID,
        smp::current_local_ptr,
        x86_64::{
            interrupts::{
                handlers::{HALT_ALL_NMI, HALTED_CPUS},
                ps2,
            },
            registers::RFLAGS,
        },
    },
    info, percpu, warn,
};

use self::gdt::init_gdt;

#[inline]
pub fn enable_sse() {
    unsafe {
        asm!(
            "
            mov rax, cr0
            and ax, 0xFFFB
            or ax, 0x2
            mov cr0, rax
            mov rax, cr4
            or ax, 3 << 9
            mov cr4, rax
        ",
        );
    }
}

#[inline]
pub fn enable_rdtsc() {
    let cr4: usize;
    unsafe {
        asm!("mov {}, cr4", out(reg) cr4);
        asm!("mov cr4, {}", in(reg) cr4 & !(1 << 2))
    }
}

#[inline]
fn _enable_avx() {
    unsafe {
        asm!(
            "
    push rax
    push rcx
    push rdx

    xor rcx, rcx
    xgetbv // Load XCR0 register
    or eax, 7 // Set AVX, SSE, X87 bits
    xsetbv // Save back to XCR0

    pop rdx
    pop rcx
    pop rax
    ret",
            options(noreturn)
        )
    }
}

/// simple init less likely to panic
/// in general memory and serial are required to be usable after this
/// highly required
#[inline]
pub fn init_phase1() {
    init_serial();
    let bsp = percpu::init_bsp_first();

    init_gdt(bsp);
    init_idt();
    tsc::calibrate_tsc();
}

pub(super) fn setup_cpu_generic2() {
    info!("enabling apic interrupts...");
    apic::enable_apic_interrupts_generic();
    unsafe {
        (*current_local_ptr()).cpu_arch_id = ArchCpuID::get();
    }
    info!("enabling apic timer...");
    apic::setup_timer();

    info!("enabling sse...");
    enable_sse();
    enable_rdtsc();
}

/// Complexer init ran after terminal initialization.
#[inline]
pub fn init_phase2() {
    setup_cpu_generic2();

    match ps2::setup_controller() {
        Ok((true, true)) => (apic::enable_apic_keyboard(), apic::enable_apic_mouse()),
        Ok((false, false)) => (warn!("No devices found in the PS/2 Controller"), ()),
        Ok((true, false)) => (apic::enable_apic_keyboard(), ()),
        Ok((false, true)) => (apic::enable_apic_mouse(), ()),
        Err(()) => (crate::error!("PS/2 Controller setup failed"), ()),
    };
}

#[inline(always)]
/// Executes a function without interrupts enabled
/// once done the interrupts status are restored (if they were disabled they'd stay disabled, if they were enabled they'd stay enabled)
/// returns whatever the function returns
///
/// # Safety
/// Safe because it restores the interrupts status once done.
pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        let interrupts_were_enabled = RFLAGS::read().interrupts_enabled();
        if interrupts_were_enabled {
            disable_interrupts();
        }

        let result = f();

        if interrupts_were_enabled {
            enable_interrupts();
        } /* otherwise keep disabled */
        result
    }
}

#[inline(always)]
#[allow(unused)]
pub fn with_interrupts<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        let interrupts_were_enabled = RFLAGS::read().interrupts_enabled();
        if !interrupts_were_enabled {
            enable_interrupts();
        }

        let result = f();

        if !interrupts_were_enabled {
            disable_interrupts();
        } /* otherwise keep disabled */
        result
    }
}

#[inline(always)]
unsafe fn disable_interrupts() {
    unsafe { core::arch::asm!("cli") }
}

#[inline(always)]
unsafe fn enable_interrupts() {
    unsafe { core::arch::asm!("sti") }
}

#[inline(always)]
pub unsafe fn hlt() {
    unsafe { core::arch::asm!("hlt") }
}

pub unsafe fn halt_all() {
    let cpus_count = crate::smp::READY_CPUS.load(Ordering::SeqCst);
    apic::send_nmi_all(HALT_ALL_NMI);
    HALTED_CPUS.fetch_add(1, Ordering::SeqCst);
    while cpus_count > HALTED_CPUS.load(Ordering::Relaxed) {
        core::hint::spin_loop();
    }
}
