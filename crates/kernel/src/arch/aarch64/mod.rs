use core::arch::{asm, global_asm};

mod cpu;
mod exceptions;
mod gic;
pub mod paging;
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
kboot:
    # parks all cores except for core 0
    mrs x1, mpidr_el1
    and x1, x1, #3
    cmp x1, #0
    bne khalt

    mov x0, sp
    # Enables SP_ELx
    mrs x1, spsel
    orr x1, x1, #1
    msr spsel, x1
    # Restores the stack back after enabling
    mov sp, x0

    b kstart
"
);

/// Switches to el1
fn switch_to_el1() {
    let current_el: usize;
    unsafe { asm!("mrs {0:x}, CurrentEl", out(reg) current_el) }

    let current_el = (current_el >> 2) & 0b11;

    if current_el != 1 {
        todo!("switch to el1 from {}", current_el)
    }
}

#[inline(always)]
pub fn init_phase1() {
    switch_to_el1();
    exceptions::init_exceptions();

    cpu::init();
    unsafe {
        enable_interrupts();
    }
}

#[inline(never)]
pub fn init_phase2() {
    gic::init_gic();
    timer::init_generic_timer();
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
    asm!("wfe");
}
