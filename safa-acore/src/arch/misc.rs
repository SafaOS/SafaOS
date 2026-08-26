use super::inner;
pub use inner::registers::ArchCpuID;

/// Halts execution (stops the CPU) until the next interrupt.
#[inline(always)]
pub fn halt() {
    inner::hlt()
}

#[inline(always)]
/// Executes a function `f` with interrupts disabled, then restores the previous interrupt flags.
pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    inner::without_interrupts(f)
}

#[allow(unused)]
/// Executes a function `f` with interrupts enabled, then restores the previous interrupts flags.
pub fn with_interrupts<R>(f: impl FnOnce() -> R) -> R {
    inner::with_interrupts(f)
}
