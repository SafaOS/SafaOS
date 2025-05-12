use crate::{cross_println, globals::KERNEL_ELF};
use core::arch::{asm, global_asm};

mod exceptions;
pub mod paging;
#[path = "../unsupported/power.rs"]
pub(super) mod power;
pub(super) mod serial;
#[path = "../unsupported/threading.rs"]
pub(super) mod threading;
#[path = "../unsupported/utils.rs"]
pub(super) mod utils;

mod registers;

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
    unsafe {
        enable_interrupts();
    }
}

#[inline(always)]
pub fn init_phase2() {}

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
// FIXME: this can be simplified by implementing some sort of stack-frame type,,,
pub fn print_stack_trace() {
    let mut fp: *const usize;

    unsafe {
        core::arch::asm!("mov {}, fp", out(reg) fp);

        cross_println!("\x1B[38;2;0;0;200mStack trace:");
        while !fp.is_null() && fp.is_aligned() {
            let return_address_ptr = fp.offset(1);
            let return_address = *return_address_ptr;

            let name = {
                let sym = KERNEL_ELF.sym_from_value_range(return_address);
                sym.and_then(|sym| KERNEL_ELF.string_table_index(sym.name_index))
            };
            let name = name.as_deref().unwrap_or("???");
            cross_println!("  {:#x} <{}>", return_address, name);
            fp = *fp as *const usize;
        }
        cross_println!("\x1B[0m");
    }
}
