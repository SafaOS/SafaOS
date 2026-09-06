use core::fmt::{Arguments, Write};

use crate::{arch::serial::Serial, logging::LoggingSink, oninit, sync::SpinLockIrq};

static SERIAL: SpinLockIrq<Serial> = SpinLockIrq::new(Serial::new());

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        Ok(self.write_str(s))
    }
}

#[doc(hidden)]
pub fn _serial(args: Arguments<'_>) {
    SERIAL
        .lock_no_irq(|mut guard| guard.write_fmt(args))
        .expect("Serial shall never fail")
}

pub fn init() -> Result<(), &'static str> {
    SERIAL.lock_no_irq(|mut guard| guard.init_serial())
}

#[macro_export]
/// Raw print to the serial without anything appended.
macro_rules! loggingsprint {
    ($($arg:tt)*) => ($crate::logging::serial::_serial(format_args!($($arg)*)));
}

#[macro_export]
/// Print a raw new line to the serial.
macro_rules! loggingsprintln {
    () => {
        $crate::logging::sprint!("\n")
    };
    ($($arg:tt)*) => ($crate::logging::sprint!("{}\n", format_args!($($arg)*)));
}

pub use loggingsprint;
pub use loggingsprintln;

impl LoggingSink for SpinLockIrq<Serial> {
    fn write_record(&self, record: &super::LogRecord) {
        self.lock_no_irq(|mut this| {
            this.write_fmt(format_args!(
                "[ {mins:04}:{secs:02}:{millis:03}:{micros:03} ] [\x1b[{}m {:<5?} \x1b[0m] \x1b[90m{}:\x1b[0m {}\n",
                record.level.ansii_color(),
                record.level,
                record.subject,
                record.arguments,
                mins = record.timestamp.minutes(),
                secs = record.timestamp.subminute_secs(),
                millis = record.timestamp.subsec_millis(),
                micros = record.timestamp.submilli_micros(),
            ))
            .expect("Failed to write a log to serial")
        })
    }

    unsafe fn panic_mode(&self) -> bool {
        unsafe { self.force_unlock() }
        true
    }
}

oninit::define! {
    pub(self) static _D: () = || { super::register_logger(crate::logging::LogLevel::Trace, &SERIAL); };
}
