pub mod serial;

pub fn init() {
    match serial::init() {
        Ok(()) => {}
        Err(e) => todo!("{e}"),
    }
}

use core::cell::SyncUnsafeCell;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

pub use serial::loggingsprint as sprint;
pub use serial::loggingsprintln as sprintln;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Very verbose tracing log.
    Trace = 0,
    /// a bit verbose debug log.
    Debug = 1,
    /// Info log.
    Info = 2,
    /// Warning log, something bad could happen.
    Warn = 3,
    /// Error that can be recovered from/procced.
    Error = 4,
    /// Fatal unrecoverable error.
    Fatal = 5,
}

impl LogLevel {
    pub const fn ansii_color(&self) -> u8 {
        match self {
            Self::Debug | Self::Error => 91,
            Self::Info => 92,
            Self::Warn => 93,
            Self::Fatal => 31,
            Self::Trace => 94,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LogRecord<'a> {
    pub timestamp: DurationFmt,
    pub level: LogLevel,
    pub file: &'static str,
    pub line: u32,
    pub subject: &'static str,
    pub arguments: core::fmt::Arguments<'a>,
}

pub trait LoggingSink: Send + Sync {
    fn write_record(&self, record: &LogRecord);
    /// Enter panic mode
    ///
    /// Unlocks the logger in case it was in use for example.
    ///
    /// returns false if panic_mode failed to enter, instead the logger would be muted.
    unsafe fn panic_mode(&self) -> bool {
        false
    }
}

enum LoggingSinkRef {
    Static(&'static dyn LoggingSink),
    None,
}

impl LoggingSink for LoggingSinkRef {
    fn write_record(&self, record: &LogRecord) {
        match self {
            Self::Static(s) => s.write_record(record),
            Self::None => {}
        }
    }

    unsafe fn panic_mode(&self) -> bool {
        match self {
            Self::Static(s) => unsafe { s.panic_mode() },
            Self::None => false,
        }
    }
}

static LOGGING_SINKS: [(AtomicU32, SyncUnsafeCell<LoggingSinkRef>); 64] =
    [const { (AtomicU32::new(0), SyncUnsafeCell::new(LoggingSinkRef::None)) }; 64];
static NEXT_SINK: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy)]
pub struct OwnedSinkID(usize);

/// Logs at or above [`loglevel`] will be given to the given logger.
///
/// logger must be static. it cannot be removed for now.
pub fn register_logger(loglevel: LogLevel, sink: &'static dyn LoggingSink) -> Option<OwnedSinkID> {
    let index = NEXT_SINK.fetch_add(1, Ordering::Relaxed);

    if index >= LOGGING_SINKS.len() {
        _ = NEXT_SINK.compare_exchange(
            index,
            LOGGING_SINKS.len(),
            Ordering::Acquire,
            Ordering::Relaxed,
        );
        return None;
    }

    let meta = loglevel as u32 + 1;
    unsafe { *LOGGING_SINKS[index].1.get() = LoggingSinkRef::Static(sink) };
    LOGGING_SINKS[index].0.store(meta, Ordering::Release);
    Some(OwnedSinkID(index))
}

/// Stops a given logger from receiving logs, this function doesn't gurantuee imeddiate effect.
pub fn logger_mute(id: OwnedSinkID) {
    LOGGING_SINKS[id.0].0.store(0, Ordering::Release);
}

/// Sets a given logger's min log level.
pub fn logger_set_loglevel(id: OwnedSinkID, loglevel: LogLevel) {
    LOGGING_SINKS[id.0]
        .0
        .store(loglevel as u32 + 1, Ordering::Release);
}

impl<'a> LogRecord<'a> {
    pub fn log(&self) {
        let mut amount_of_sinks = NEXT_SINK.load(Ordering::Relaxed);

        for (meta, sink) in LOGGING_SINKS.iter() {
            if amount_of_sinks == 0 {
                return;
            }

            let meta = meta.load(Ordering::Acquire);
            if meta == 0 || meta - 1 > self.level as u32 {
                continue;
            }

            unsafe { (*sink.get()).write_record(self) };
            amount_of_sinks -= 1;
        }
    }
}

/// Enters panic mode
///
/// All CPUs must be stopped first
pub unsafe fn panic_mode() {
    for (idx, (meta, sink)) in LOGGING_SINKS.iter().enumerate() {
        let meta = meta.load(Ordering::Acquire);
        if meta == 0 {
            continue;
        }

        if !unsafe { (*sink.get()).panic_mode() } {
            logger_mute(OwnedSinkID(idx));
        }
    }
}

#[macro_export]
macro_rules! _generic_log_macro {
    ($level: expr, $subject: expr, $($arg:tt)*) => {
        let (file, line) = (file!(), line!());
        $crate::logging::LogRecord {
            timestamp: $crate::time::DurationFmt::new($crate::time::time_since_boot()),
            level: $level,
            file,
            line,
            subject: $subject,
            arguments: format_args!($($arg)*),
        }.log()
    };
}

/// Logs a [`LogLevel::Debug`]-level message with the given subject and arguments.
#[macro_export]
macro_rules! _loggerdebug {
    ($subject: ty, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Debug, stringify!($subject), $($arg)*));
    ($subject: literal, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Debug, $subject, $($arg)*));
}

/// Logs a [`LogLevel::Trace`]-level message with the given subject and arguments.
#[macro_export]
macro_rules! _loggertrace {
    ($subject: ty, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Trace, stringify!($subject), $($arg)*));
    ($subject: literal, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Trace, $subject, $($arg)*));
}

#[macro_export]
/// Logs a [`LogLevel::Fatal`]-level message with the given subject and arguments.
macro_rules! _loggerfatal {
    ($subject: ty, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Fatal, stringify!($subject), $($arg)*));
    ($subject: literal, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Fatal, $subject, $($arg)*));
}

#[macro_export]
/// Logs a [`LogLevel::Error`]-level message with the given subject and arguments.
macro_rules! _loggererror {
    ($subject: ty, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Error, stringify!($subject), $($arg)*));
    ($subject: literal, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Error, $subject, $($arg)*));
}

/// Logs a [`LogLevel::Info`]-level message with the given subject and arguments.
#[macro_export]
macro_rules! _loggerinfo {
    ($subject: ty, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Info, stringify!($subject), $($arg)*));
    ($subject: literal, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Info, $subject, $($arg)*));
}

/// Logs a [`LogLevel::Warn`]-level message with the given subject and arguments.
#[macro_export]
macro_rules! _loggerwarn {
    ($subject: ty, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Warn, stringify!($subject), $($arg)*));
    ($subject: literal, $($arg:tt)*) => ($crate::logging::_generic_log_macro!($crate::logging::LogLevel::Warn, $subject, $($arg)*));
}

pub use _generic_log_macro;
pub use _loggerdebug as debug;
pub use _loggererror as error;
pub use _loggerfatal as fatal;
pub use _loggerinfo as info;
pub use _loggertrace as trace;
pub use _loggerwarn as warn;

use crate::time::DurationFmt;
