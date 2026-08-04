use core::arch::{asm, global_asm};

use crate::arch::aarch64::exceptions::HALT_ALL_SGI;

mod cpu;
mod exceptions;
mod gic;
pub(super) mod interrupts;
pub(super) mod io;
pub mod paging;
pub(super) mod pci;
pub(super) mod power;
pub(super) mod registers;
pub(super) mod serial;
pub(super) mod smp;
#[cfg(test)]
mod tests;
pub(super) mod threading;
mod timer;
pub(super) mod tlb;
pub(super) mod utils;

global_asm!(
    "
.text
.global kboot
.global stack_init

stack_init:
    mov x0, sp
    # Enables SP_ELx
    mrs x1, spsel
    orr x1, x1, #1
    msr spsel, x1
    # Restores the stack back after enabling
    mov sp, x0
    ret

# boots core 0
kboot:
    b kstart
"
);

unsafe extern "C" {
    fn stack_init();
}

/// Switches to el1
fn switch_to_el1() {
    let current_el: usize;
    unsafe { asm!("mrs {0:x}, CurrentEl", out(reg) current_el) }

    let current_el = (current_el >> 2) & 0b11;

    if current_el != 1 {
        todo!("switch to el1 from {}", current_el)
    }
}

fn enable_user_timer() {
    unsafe {
        asm!(
            "
            mov x0, #0b1100000011
            mrs x1, CNTKCTL_EL1
            orr x0, x0, x1
            msr CNTKCTL_EL1, x0
            ",
            out("x1") _,
            out("x0") _,
        );
    }
}
fn enable_fp() {
    unsafe {
        asm!(
            "
            # No trap to all NEON & FP instructions
            mov x0, #0x00300000
            mrs x1, CPACR_EL1
            orr x0, x0, x1
            msr CPACR_EL1, X0
            ",
            out("x1") _,
            out("x0") _,
        )
    }
}
#[inline(always)]
fn setup_cpu_basics() {
    unsafe {
        stack_init();
    }
    switch_to_el1();
    exceptions::init_exceptions();
    enable_fp();
    enable_user_timer();
}

fn setup_cpu_pherphials() {
    gic::gic_init_cpu();
    timer::setup_generic_timer();
}

#[inline(always)]
pub fn init_phase1() {
    setup_cpu_basics();
    serial::init_serial_qemu();
    let bsp = crate::percpu::init_bsp_first();
    smp::setup_cpu_mp(bsp);
}

#[inline(never)]
pub fn init_phase2() {
    gic::init_gic();
    timer::init_generic_timer();
    setup_cpu_pherphials();
    HALT_ALL_SGI.clear_pending_all().enable_all();
}

#[inline(always)]
pub(super) fn get_daif() -> u64 {
    let results: u64;
    unsafe { asm!("mrs {:x}, DAIF", out(reg) results) };
    results
}

#[inline(always)]
pub(super) fn set_daif(value: u64) {
    unsafe { asm!("msr DAIF, {:x}", in(reg) value) }
}

#[inline(always)]
pub unsafe fn disable_interrupts() {
    unsafe { asm!("msr DAIFSet, #0b1111") }
}

/// Executes a function without interrupts enabled
/// once done the interrupts status are restored (if they were disabled they'd stay disabled, if they were enabled they'd stay enabled)
/// returns whatever the function returns
///
/// # Safety
/// Safe because it restores the interrupts status once done.
pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let daif = get_daif();
    unsafe {
        disable_interrupts();
    }

    let result = f();

    set_daif(daif);
    result
}

#[allow(unused)]
pub fn with_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let daif = get_daif();
    unsafe {
        enable_interrupts();
    }

    let result = f();

    set_daif(daif);
    result
}

#[inline(always)]
pub unsafe fn enable_interrupts() {
    unsafe { asm!("msr DAIFClr, #0b1111") }
}

/// Halts all CPUs
#[inline(always)]
pub unsafe fn halt_all() {
    // let cpus_len = CpuLocal::get_all().len_hint();
    HALT_ALL_SGI.request_sgi_all(true);

    // while exceptions::HALT_RESPONSE.load(Ordering::Relaxed) < cpus_len - 1 {
    //     core::hint::spin_loop();
    // }
}

#[inline(always)]
pub unsafe fn hlt() {
    unsafe {
        asm!("wfe");
    }
}
