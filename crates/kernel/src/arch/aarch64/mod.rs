use crate::{cross_println, khalt};
use core::arch::asm;

pub mod paging;
#[path = "../unsupported/power.rs"]
pub(super) mod power;
pub(super) mod serial;
#[path = "../unsupported/threading.rs"]
pub(super) mod threading;
#[path = "../unsupported/utils.rs"]
pub(super) mod utils;

/// parks all cores except for 0
fn park_cores() {
    let mut cpu_id: usize;
    unsafe {
        asm!("
               mrs x1, mpidr_el1
               and x1, x1, #3
               mov {}, x1
               ", out(reg) cpu_id);

        if cpu_id > 0 {
            khalt()
        }
    }
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

#[inline(always)]
pub fn init_phase1() {
    park_cores();
    switch_to_el1();
}

#[inline(always)]
pub fn init_phase2() {}

#[inline(always)]
pub unsafe fn disable_interrupts() {}

#[inline(always)]
pub unsafe fn enable_interrupts() {}

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

            let name = "??";

            cross_println!("  {:#x} <{}>", return_address, name);
            fp = *fp as *const usize;
        }
        cross_println!("\x1B[0m");
    }
}
