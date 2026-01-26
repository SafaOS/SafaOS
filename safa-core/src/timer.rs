use core::{fmt::Display, time::Duration};

use crate::arch;

/// The number of nanoseconds per second
pub const NANO_SECONDS_PER_SECOND: u32 = 10u32.pow(9);

/// The instant at which the system booted.
pub const BOOT_INSTANT: SystemInstant = SystemInstant { cpu_cycles: 0 };

/// Describes a unique instant in time.
#[derive(Debug, Clone, Copy)]
pub struct SystemInstant {
    cpu_cycles: u64,
}

impl SystemInstant {
    /// Retrieves the current instant.
    #[inline(always)]
    pub fn now() -> Self {
        Self {
            cpu_cycles: arch::utils::cpu_cycles(),
        }
    }

    #[inline]
    /// Returns the duration elapsed since the instant.
    pub fn elapsed(&self) -> Duration {
        self.elapsed_from(&Self::now())
    }

    /// Returns the duration of time elapsed since the instant `other`.
    pub fn elapsed_from(&self, other: &Self) -> Duration {
        let frequency_mhz = arch::utils::cpu_timer_freq_mhz();
        let cycles = self.cpu_cycles.abs_diff(other.cpu_cycles);

        let total_nanos = (cycles * (NANO_SECONDS_PER_SECOND / 1000 / 1000) as u64) / frequency_mhz;

        let seconds = total_nanos / NANO_SECONDS_PER_SECOND as u64;
        let sub_nano_seconds = (total_nanos % NANO_SECONDS_PER_SECOND as u64) as u32;

        Duration::new(seconds, sub_nano_seconds)
    }
}

/// Display formats a duration
#[derive(Clone, Copy)]
pub struct DurationFmt(Duration);
impl DurationFmt {
    /// Constructs a new duration formatter
    ///
    /// TODO: implement configurations
    pub const fn new(duration: Duration) -> Self {
        Self(duration)
    }
}

impl Display for DurationFmt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let seconds = self.0.as_secs();
        let millis = self.0.subsec_millis();
        let micros = self.0.subsec_micros() % 1000;

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

/// Returns the amount of milliseconds passed since boot
///
/// equalivent to [`BOOT_INSTANT`].elapsed().as_millis()
#[inline]
pub fn time_since_boot_ms() -> u64 {
    BOOT_INSTANT.elapsed().as_millis() as u64
}
