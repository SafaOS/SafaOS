use crate::bootloader;
use core::num::NonZero;

#[inline]
pub fn cpu_timer_freq_mhz() -> NonZero<u64> {
    NonZero::new(
        bootloader::tsc_freq_hz()
            .expect("TODO: Manual TSC calibration")
            .get()
            / 1_000_000u64,
    )
    .expect("TSC Frequency mhz shouldn't be 0")
}

#[inline(always)]
pub fn cpu_timer_ticks() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}
