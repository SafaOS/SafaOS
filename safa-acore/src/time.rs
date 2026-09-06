use core::{fmt::Display, time::Duration};

use crate::arch;

/// The number of nanoseconds per second
pub const NANO_SECONDS_PER_SECOND: u32 = 10u32.pow(9);

/// The instant at which the system booted.
pub const BOOT_INSTANT: SystemInstant = SystemInstant { tsc_ticks: 0 };

/// Describes a unique instant in time.
#[derive(Debug, Clone, Copy)]
pub struct SystemInstant {
    tsc_ticks: u64,
}

impl SystemInstant {
    /// Retrieves the current instant.
    #[inline(always)]
    pub fn now() -> Self {
        Self {
            tsc_ticks: arch::timers::cpu_timer_ticks(),
        }
    }

    #[inline]
    /// Returns the duration elapsed since the instant.
    pub fn elapsed(&self) -> Duration {
        self.elapsed_from(&Self::now())
    }

    /// Returns the duration of time elapsed since the instant `other`.
    pub fn elapsed_from(&self, other: &Self) -> Duration {
        let frequency_mhz = arch::timers::cpu_timer_freq_mhz();
        let cycles = self.tsc_ticks.abs_diff(other.tsc_ticks);

        let total_nanos = (cycles * (NANO_SECONDS_PER_SECOND / 1000 / 1000) as u64) / frequency_mhz;

        let seconds = total_nanos / NANO_SECONDS_PER_SECOND as u64;
        let sub_nano_seconds = (total_nanos % NANO_SECONDS_PER_SECOND as u64) as u32;

        Duration::new(seconds, sub_nano_seconds)
    }
}

/// Display formats a duration
#[derive(Debug, Clone, Copy)]
pub struct DurationFmt(Duration);
impl DurationFmt {
    /// Constructs a new duration formatter
    ///
    /// TODO: implement configurations
    pub const fn new(duration: Duration) -> Self {
        Self(duration)
    }

    #[inline(always)]
    /// Returns the number of minutes passed.
    pub const fn minutes(&self) -> u64 {
        self.secs() / 60
    }

    #[inline(always)]
    /// Returns the overflow of seconds passed since minutes.
    pub const fn subminute_secs(&self) -> u64 {
        self.secs() % 60
    }

    /// Returns number of seconds elapsed in duration.
    #[inline(always)]
    pub const fn secs(&self) -> u64 {
        self.0.as_secs()
    }

    /// Returns the overflow of milliseconds elapsed in a duration.
    #[inline(always)]
    pub const fn subsec_millis(&self) -> u32 {
        self.0.subsec_millis()
    }

    /// Returns the overflow of microseconds elapsed relative to [`Self::subsec_millis`] (sub millisecond).
    #[inline(always)]
    pub const fn submilli_micros(&self) -> u32 {
        self.0.subsec_micros() % 1000
    }
}

impl Display for DurationFmt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let seconds = self.secs();
        let millis = self.subsec_millis();
        let micros = self.submilli_micros();

        if seconds != 0 {
            write!(f, "{seconds}s")?;
        }

        if millis != 0 {
            if seconds != 0 {
                write!(f, ":")?;
            }
            write!(f, "{millis}ms")?;
        }

        if micros != 0 {
            if millis != 0 || seconds != 0 {
                write!(f, ":")?;
            }
            write!(f, "{micros}us")?;
        }
        Ok(())
    }
}

#[inline]
/// Returns the amount of time passed since boot
pub fn time_since_boot() -> Duration {
    BOOT_INSTANT.elapsed()
}
