mod acpi;
mod gdt;
pub(super) mod interrupts;
pub mod paging;
pub(super) mod pci;
pub(super) mod power;
pub(super) mod registers;
pub(super) mod serial;
mod syscalls;
#[cfg(test)]
mod tests;
pub(super) mod threading;
pub(super) mod utils;

use core::arch::asm;
use interrupts::{apic, init_idt};
use serial::init_serial;

use crate::{
    arch::x86_64::{
        gdt::TaskStateSegment,
        interrupts::handlers::{FLUSH_CACHE_ALL_ID, HALT_ALL_HANDLER_ID},
        registers::RFLAGS,
        utils::TICKS_PER_MS,
    },
    info, sleep,
    utils::locks::SpinMutex,
};

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
    // CPU 0 is initialized in a special way
    _ = setup_cpu_generic0();
}
#[must_use = "returns a pointer to the TSS of the current CPU, this pointer must be stored in the CPU Local Storage"]
pub(super) fn setup_cpu_generic0() -> *mut TaskStateSegment {
    let tss = init_gdt();
    init_idt();
    tss
}

pub(super) fn setup_cpu_generic1(tsc_ticks_per_ms: &mut u64) {
    info!("enabling apic interrupts...");
    apic::enable_apic_interrupts_generic(tsc_ticks_per_ms);
    info!("enabling sse...");
    enable_sse();
}
/// Complexer init ran after terminal initialization.
#[inline]
pub fn init_phase2() {
    setup_cpu_generic1(unsafe { &mut *TICKS_PER_MS.get() });
    apic::enable_apic_keyboard();
}

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

pub unsafe fn flush_cache_inner() {
    unsafe {
        // TODO: use INVLPG
        core::arch::asm!(
            "
            mov rax, cr3
            mov cr3, rax
            ",
        )
    }
}

pub unsafe fn flush_cache() {
    static _CACHE_FLUSH: SpinMutex<()> = SpinMutex::new(());
    let _guard = _CACHE_FLUSH.lock();
    unsafe {
        flush_cache_inner();
    }
    apic::send_nmi_all(FLUSH_CACHE_ALL_ID);
}

pub unsafe fn halt_all() {
    apic::send_nmi_all(HALT_ALL_HANDLER_ID);
    sleep!(100 ms)
}
