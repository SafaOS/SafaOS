use core::arch::asm;
pub mod boot;
// pub mod paging;
pub mod registers;
pub mod serial;

#[inline(always)]
pub fn hlt() {
    unsafe { asm!("wfi") }
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

#[inline(always)]
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
