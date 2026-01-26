use core::{cell::SyncUnsafeCell, num::NonZero};

use crate::{arch::x86_64::pit, serial, utils::locks::SpinLock};

pub static TSC_FREQ_MHZ: SyncUnsafeCell<NonZero<u64>> = SyncUnsafeCell::new(NonZero::<u64>::MAX);

/// Calibrates and initializes the TSC
pub fn calibrate_tsc() {
    static _CALIBRATE_LOCK: SpinLock<()> = SpinLock::new(());
    let _guard = _CALIBRATE_LOCK.lock();
    serial!("calibrating tsc\n");
    unsafe {
        let freq = pit::calibrate_tsc();
        serial!("calibrated TSC with {} ticks in 1us", freq);
        *TSC_FREQ_MHZ.get() = freq;
    }
}

/// Returns the current value of the TSC
#[inline(always)]
pub fn read_tsc() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
pub fn tsc_freq_mhz() -> NonZero<u64> {
    unsafe { *TSC_FREQ_MHZ.get() }
}
