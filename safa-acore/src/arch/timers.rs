use super::inner;
use core::num::NonZero;

/// Returns the frequency of the CPU TSC in mega-hertz.
#[inline(always)]
pub fn cpu_timer_freq_mhz() -> NonZero<u64> {
    inner::timers::cpu_timer_freq_mhz()
}

/// Returns the ticks that passed since the CPU TSC started counting.
#[inline(always)]
pub fn cpu_timer_ticks() -> u64 {
    inner::timers::cpu_timer_ticks()
}
