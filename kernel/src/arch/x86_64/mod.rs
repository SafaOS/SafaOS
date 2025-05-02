mod acpi;
pub mod gdt;
pub mod interrupts;
pub mod power;
pub mod serial;
mod syscalls;
pub mod threading;
pub mod utils;

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

/// Complexer init ran after terminal initilization.
#[inline]
pub fn init_phase2() {
    info!("enabling apic interrupts...");
    apic::enable_apic_interrupts();
    info!("enabling sse...");
    enable_sse();
}
