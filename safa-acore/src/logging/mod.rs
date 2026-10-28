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

#[allow(unused)]
pub use crate::terminal::__tprint as tprint;
#[allow(unused)]
pub use crate::terminal::__tprintln as tprintln;

pub use libakernel::logging::*;

#[unsafe(no_mangle)]
extern "C" fn SafaLogger_log(record: &LogRecord) {
    let mut amount_of_sinks = NEXT_SINK.load(Ordering::Relaxed);

    for (meta, sink) in LOGGING_SINKS.iter() {
        if amount_of_sinks == 0 {
            return;
        }

        let meta = meta.load(Ordering::Acquire);
        if meta == 0 || meta - 1 > record.level as u32 {
            continue;
        }

        unsafe { (*sink.get()).write_record(record) };
        amount_of_sinks -= 1;
    }
}
pub use serial::loggingsprint as sprint;
pub use serial::loggingsprintln as sprintln;

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
#[allow(unused)]
pub fn logger_set_loglevel(id: OwnedSinkID, loglevel: LogLevel) {
    LOGGING_SINKS[id.0]
        .0
        .store(loglevel as u32 + 1, Ordering::Release);
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
