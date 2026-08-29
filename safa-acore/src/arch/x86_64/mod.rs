pub mod boot;
pub mod gdt;
pub mod io;
pub mod paging;
pub mod registers;

pub(crate) mod serial;

use registers::RFLAGS;

pub fn hlt() {
    unsafe { core::arch::asm!("hlt") }
}

#[inline(always)]
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
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) }
}

#[inline(always)]
unsafe fn enable_interrupts() {
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) }
}
