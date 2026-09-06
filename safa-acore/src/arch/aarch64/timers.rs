use core::num::NonZero;

#[inline]
pub fn cpu_timer_freq_mhz() -> NonZero<u64> {
    let freq: u64;
    unsafe {
        core::arch::asm!(
            "mrs {frq}, cntfrq_el0",
            frq = out(reg) freq,
        );
    }

    unsafe { NonZero::new_unchecked(freq / 1_000_000) }
}

#[inline(always)]
/// Returns the number of CPU TSC ticks since the CPU was started
pub fn cpu_timer_ticks() -> u64 {
    let count: u64;
    unsafe {
        core::arch::asm!(
            "isb",
            "mrs {cnt}, cntpct_el0",
            cnt = out(reg) count,
        );
    }
    count
}
