mod acpi;
pub mod gdt;
pub mod interrupts;
pub mod paging;
pub mod power;
pub mod serial;
mod syscalls;
#[cfg(test)]
mod tests;
pub mod threading;
pub mod utils;

use crate::cross_println;
use crate::globals::KERNEL_ELF;
use core::arch::asm;
use interrupts::{apic, init_idt};
use serial::init_serial;

use crate::info;

use self::gdt::init_gdt;

pub fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}

pub fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

pub fn outw(port: u16, value: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
    }
}

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
            options(nostack)
        )
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
    init_gdt();
    init_idt();
}

/// Complexer init ran after terminal initialization.
#[inline]
pub fn init_phase2() {
    info!("enabling apic interrupts...");
    apic::enable_apic_interrupts();
    info!("enabling sse...");
    enable_sse();
}

#[inline(always)]
pub unsafe fn disable_interrupts() {
    unsafe { core::arch::asm!("cli") }
}

#[inline(always)]
pub unsafe fn enable_interrupts() {
    unsafe { core::arch::asm!("sti") }
}

#[inline(always)]
pub unsafe fn hlt() {
    unsafe { core::arch::asm!("hlt") }
}

#[allow(unused)]
pub fn print_stack_trace() {
    let mut fp: *const usize;

    unsafe {
        core::arch::asm!("mov {}, rbp", out(reg) fp);

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
