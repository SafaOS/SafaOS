#[allow(unused)]
// Used by tests and should be used by scheduler in the future.
pub use libkernel::time::*;

#[unsafe(no_mangle)]
extern "C" fn arch_cpu_timer_ticks() -> u64 {
    crate::arch::timers::cpu_timer_ticks()
}
#[unsafe(no_mangle)]
extern "C" fn arch_cpu_timer_freq_mhz() -> core::num::NonZero<u64> {
    crate::arch::timers::cpu_timer_freq_mhz()
}
