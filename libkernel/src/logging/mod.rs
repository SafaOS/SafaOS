use core::{
    fmt::{Arguments, Display},
    marker::PhantomData,
};

use crate::{ffi::FFIStr, time::DurationFmt};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// A bad implementation of an Rust-To-Rust FFI Safe [`Arguments`].
pub struct FFIArguments<'a> {
    arguments: *const (),
    argument_write: extern "C" fn(Self, *mut (), extern "C" fn(*mut (), FFIStr) -> bool) -> bool,
    _layout: PhantomData<&'a Arguments<'a>>,
}

impl<'a> Display for FFIArguments<'a> {
    #[inline(always)]
    fn fmt<'f>(&self, f: &mut core::fmt::Formatter<'f>) -> core::fmt::Result {
        extern "C" fn write_str_to_fmt(fmt: *mut (), str: FFIStr) -> bool {
            unsafe { &mut *fmt.cast::<core::fmt::Formatter>() }
                .write_str(&*str)
                .is_ok()
        }

        if !(self.argument_write)(*self, f as *mut _ as *mut (), write_str_to_fmt) {
            Err(core::fmt::Error)
        } else {
            Ok(())
        }
    }
}

impl<'a> FFIArguments<'a> {
    #[inline(always)]
    /// Constructs a new FFI Arguments from given arguments.
    ///
    /// This is safe to send to other versions of rust where the layout of Arguments may differ.
    ///
    /// FIXME: This may not be safe in an unwinding panic target?
    pub const fn new(args: &'a Arguments<'a>) -> Self {
        extern "C" fn format_the_args<'a>(
            this: FFIArguments<'a>,
            private: *mut (),
            write_str: extern "C" fn(*mut (), FFIStr) -> bool,
        ) -> bool {
            let arguments: &'a Arguments<'a> =
                unsafe { &*(this.arguments as *const Arguments<'a>) };

            struct Writer {
                private: *mut (),
                write_str: extern "C" fn(*mut (), FFIStr) -> bool,
            }

            use core::fmt::Write;
            impl Write for Writer {
                #[inline(always)]
                fn write_str(&mut self, s: &str) -> core::fmt::Result {
                    if !(self.write_str)(self.private, FFIStr::from_str(s)) {
                        Err(core::fmt::Error)
                    } else {
                        Ok(())
                    }
                }
            }

            { Writer { private, write_str }.write_fmt(*arguments) }.is_ok()
        }

        Self {
            arguments: args as *const Arguments<'a> as *const (),
            argument_write: format_the_args,
            _layout: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;
    #[test]
    pub fn ffi_arguments() {
        use core::fmt::Write;
        let arguments = format_args!("Static string");
        let arguments = FFIArguments::new(&arguments);

        let mut string = String::new();
        write!(&mut string, "{arguments}").expect("Failed to write arguments");

        let arguments = format_args!(
            " A simple format: {},{}n",
            core::hint::black_box(25),
            String::from("5")
        );

        let arguments = FFIArguments::new(&arguments);
        write!(&mut string, "{arguments}").expect("Failed to write arguments");

        assert_eq!(string, "Static string A simple format: 25,5n");
    }
}

unsafe extern "C" {
    fn SafaLogger_log(record: &LogRecord);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
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
    #[inline]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Error => "error",
            Self::Fatal => "fatal",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Trace => "trace",
        }
    }

    #[inline]
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
#[repr(C)]
pub struct LogRecord<'a> {
    pub timestamp: DurationFmt,
    pub level: LogLevel,
    pub file: FFIStr<'static>,
    pub line: u32,
    pub subject: FFIStr<'static>,
    pub arguments: FFIArguments<'a>,
}

impl<'a> LogRecord<'a> {
    #[inline(always)]
    pub fn log(&self) {
        unsafe { SafaLogger_log(self) };
    }
}

#[macro_export]
macro_rules! _generic_log_macro {
    ($level: expr, $subject: expr, $($arg:tt)*) => {{
        let (file, line) = (file!(), line!());
        let args = format_args!($($arg)*);
        $crate::logging::LogRecord {
            timestamp: $crate::time::DurationFmt::new($crate::time::time_since_boot()),
            level: $level,
            file: file.into(),
            line: line.into(),
            subject: ($subject).into(),
            arguments: $crate::logging::FFIArguments::new(&args),
        }.log()
    }};
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
