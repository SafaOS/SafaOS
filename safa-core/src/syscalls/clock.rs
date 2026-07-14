use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use safa_abi::clock::{CDuration, Clock};
use safa_abi::errors::ErrorStatus;

use crate::{arch, timer};

use super::ffi::*;
use macros::syscall_handler;

impl SyscallFFI for Clock {
    type Args = u32;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Clock::try_from(args).ok_or(ErrorStatus::InvalidArgument)
    }
}

static RTC_DATE_AT_BOOT: AtomicU64 = AtomicU64::new(0);

#[syscall_handler]
pub fn sysclock_gettime(clock: Clock, results: &mut CDuration) {
    let duration = match clock {
        Clock::RTC => {
            let mut date_at_boot = RTC_DATE_AT_BOOT.load(Ordering::Relaxed);
            if date_at_boot == 0 {
                date_at_boot = crate::limine::date_at_boot().as_nanos() as u64;
                loop {
                    let results = RTC_DATE_AT_BOOT.compare_exchange(
                        0,
                        date_at_boot,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );

                    match results {
                        Err(0) => continue,
                        Ok(_) | Err(_) => {
                            break;
                        }
                    }
                }
            }

            let durat_at_boot = Duration::from_nanos(date_at_boot);
            durat_at_boot + timer::BOOT_INSTANT.elapsed()
        }
        Clock::Monotonic => timer::BOOT_INSTANT.elapsed(),
    };
    *results = duration.into();
}

#[syscall_handler]
pub fn sysclock_settime(_clock: Clock, _time: &CDuration) -> Result<(), ErrorStatus> {
    // Not yet implemented.
    Err(ErrorStatus::MissingPermissions)
}

#[syscall_handler]
pub fn sysclock_getcntfreq(freq: &mut u64, _flags: u32) {
    *freq = arch::utils::cpu_timer_freq_mhz().get() * 1_000_000;
}

#[syscall_handler]
pub fn sysclock_getres(clock: Clock, results: &mut CDuration) {
    let duration = match clock {
        Clock::RTC => Duration::from_secs(1),
        Clock::Monotonic => Duration::from_micros(1),
    };

    *results = duration.into();
}

#[test_case]
pub fn test_rtc() {
    use crate::debug;
    use time::OffsetDateTime;

    let mut date = CDuration::ZERO;
    sysclock_gettime(Clock::RTC, &mut date);
    let date_now: Duration = date.into();
    let compile_date = compile_time::date!();

    let base_datetime = OffsetDateTime::UNIX_EPOCH;
    let date_now: OffsetDateTime = base_datetime + date_now;
    debug!("Compiled at: {compile_date}, date now is {date_now}");
    assert_eq!(date_now.year(), compile_date.year());
}
