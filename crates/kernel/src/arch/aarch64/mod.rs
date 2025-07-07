use core::arch::{asm, global_asm};

mod cpu;
mod exceptions;
mod gic;
pub(super) mod interrupts;
pub mod paging;
pub(super) mod pci;
pub(super) mod power;
pub(super) mod registers;
pub(super) mod serial;
#[cfg(test)]
mod tests;
pub(super) mod threading;
mod timer;
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

fn enable_fp() {
    unsafe {
        asm!(
            "
            # No trap to all NEON & FP instructions
            mov x0, #0x00300000
            mrs x1, CPACR_EL1
            orr x0, x0, x1
            msr CPACR_EL1, X0
            "
        )
    }
}
#[inline(always)]
fn setup_cpu_generic0() {
    unsafe {
        stack_init();
    }
    switch_to_el1();
    exceptions::init_exceptions();
    enable_fp();
}

fn setup_cpu_generic1() {
    gic::gic_init_cpu();
    timer::setup_generic_timer();
}

#[inline(always)]
pub fn init_phase1() {
    setup_cpu_generic0();
    cpu::init();
}

#[inline(never)]
pub fn init_phase2() {
    gic::init_gic();
    timer::init_generic_timer();
    setup_cpu_generic1();
}

#[inline(always)]
pub fn interrupts_disabled() -> bool {
    let results: u64;
    unsafe { asm!("mrs {:x}, DAIF", out(reg) results) };
    (results >> 6) & 0xFF == 0
}
#[inline(always)]
pub unsafe fn disable_interrupts() {
    unsafe { asm!("msr DAIFSet, #0b1111") }
}

#[inline(always)]
pub unsafe fn enable_interrupts() {
    unsafe { asm!("msr DAIFClr, #0b1111") }
}

#[inline(always)]
pub unsafe fn hlt() {
    unsafe {
        asm!("wfe");
    }
}

pub fn flush_cache() {
    unsafe {
        asm!(
            "
            tlbi VMALLE1
            dsb ISH
            isb
            "
        );
    }
}
