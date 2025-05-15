use core::{fmt::Display, sync::atomic::AtomicBool};

use crate::{arch::registers::StackFrame, globals::KERNEL_ELF};

pub const QUITE_PANIC: bool = true;
pub static BOOTING: AtomicBool = AtomicBool::new(false);

pub(crate) fn log_time_from_ms(ms: u64) -> (u32, u8, u8, u16) {
    let into_seconds = || (ms / 1000, ms % 1000);
    let into_minutes = || {
        let (seconds, ms) = into_seconds();
        (seconds / 60, seconds % 60, ms)
    };
    let into_hours = || {
        let (minutes, seconds, ms) = into_minutes();
        (minutes / 60, minutes % 60, seconds, ms)
    };

    match ms {
        ..1000 => (0, 0, 0, ms as u16),
        1000..60000 => {
            let (seconds, ms) = into_seconds();
            (0, 0, seconds as u8, ms as u16)
        }
        x if x <= 1000 * 60 * 60 && x >= 1000 * 60 => {
            let (minutes, seconds, ms) = into_minutes();
            (0, minutes as u8, seconds as u8, ms as u16)
        }
        _ => {
            let (hours, minutes, seconds, ms) = into_hours();
            (hours as u32, minutes as u8, seconds as u8, ms as u16)
        }
    }
}

#[macro_export]
macro_rules! serial_log {
    ($($arg:tt)*) => {
        let log_time = $crate::time!();
        let (hours, minutes, seconds, ms) = $crate::logging::log_time_from_ms(log_time);
        $crate::serial!("[{hours:02}:{minutes:02}:{seconds:02}.{ms:03}] {}\n", format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! tty_log {
    ($($arg:tt)*) => {
        let log_time = $crate::time!();
        let (hours, minutes, seconds, ms) = $crate::logging::log_time_from_ms(log_time);
        $crate::println!("[{hours:02}:{minutes:02}:{seconds:02}.{ms:03}] {}", format_args!($($arg)*));
    };
}

/// prints to both the serial and the terminal doesn't print to the terminal if it panicked or if
/// it is not ready...
#[macro_export]
macro_rules! panic_println {
    ($($arg:tt)*) => {
        panic_print!("{}\n", format_args!($($arg)*));
    };
}

/// prints to both the serial and the terminal doesn't print to the terminal if it panicked or if
/// it is not ready...
#[macro_export]
macro_rules! panic_print {
    ($($arg:tt)*) => {
        $crate::serial!($($arg)*);

        if !$crate::logging::QUITE_PANIC {
            $crate::print!($($arg)*);
        }
    };
}

#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::tty_log!("{}", format_args!($($arg)*));
        $crate::serial_log!("{}", format_args!($($arg)*));
    };
}

/// logs line to the TTY only when the kernel is initializing
/// logs to the serial in all cases
#[macro_export]
macro_rules! logln_boot {
    ($($arg:tt)*) => {
        if $crate::logging::BOOTING.load(core::sync::atomic::Ordering::Relaxed) {
            $crate::tty_log!("{}", format_args!($($arg)*));
        }
        $crate::serial_log!("{}", format_args!($($arg)*));
    };
}

/// runtime debug info that is only available though test feature
/// takes a $mod and an Arguments, mod must be a type
#[macro_export]
macro_rules! debug {
    ($mod: path, $($arg:tt)*) => {
        // makes sure $mod is a valid type
        let _ = core::marker::PhantomData::<$mod>;
        $crate::logln_boot!("\x1B[0m[ \x1B[91m debug \x1B[0m ]\x1B[90m {}:\x1B[0m {}", stringify!($mod), format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::logln!("[ \x1B[92m info \x1B[0m  ]\x1b[90m:\x1B[0m {}", format_args!($($arg)*));
    };
}

#[derive(Clone, Copy)]
pub struct StackTrace<'a>(&'a StackFrame);

impl<'a> StackTrace<'a> {
    /// Gets the current Stack Trace, unsafe because the StackTrace may be corrupted
    #[inline(always)]
    pub unsafe fn current() -> Self {
        Self(unsafe { StackFrame::get_current() })
    }
}

impl<'a> Display for StackTrace<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe {
            let mut fp = self.0;
            writeln!(f, "\x1B[34mStack trace:")?;
            loop {
                let return_address = fp.return_ptr();

                let name = {
                    let sym = KERNEL_ELF.sym_from_value_range(return_address as usize);
                    sym.and_then(|sym| KERNEL_ELF.string_table_index(sym.name_index))
                };
                let name = name.as_deref().unwrap_or("???");
                writeln!(f, "  {:?} <{}>", return_address, name)?;

                let Some(frame) = fp.prev() else {
                    break;
                };

                fp = frame;
            }
            write!(f, "\x1B[0m")?;
        }
        Ok(())
    }
}
