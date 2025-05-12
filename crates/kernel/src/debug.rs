use core::{fmt::Display, sync::atomic::AtomicBool};

use crate::{arch::registers::StackFrame, globals::KERNEL_ELF};

pub const QUITE_PANIC: bool = true;
pub static BOOTING: AtomicBool = AtomicBool::new(false);

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

        if !$crate::debug::QUITE_PANIC {
            $crate::print!($($arg)*);
        }
    };
}

#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::println!("{}", format_args!($($arg)*));
        $crate::serial!("{}\n", format_args!($($arg)*));
    };
}

/// logs line to the TTY only when the kernel is initializing
/// logs to the serial in all cases
#[macro_export]
macro_rules! logln_boot {
    ($($arg:tt)*) => {
        if $crate::debug::BOOTING.load(core::sync::atomic::Ordering::Relaxed) {
            $crate::println!("{}", format_args!($($arg)*));
        }
        $crate::serial!("{}\n", format_args!($($arg)*));
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
