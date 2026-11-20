use core::fmt::Display;

use crate::{arch::registers::StackFrame, symbols::KERNEL_SYMS};

#[derive(Clone, Copy)]
pub struct StackTrace<'a>(StackFrame<'a>);

impl<'a> StackTrace<'a> {
    #[allow(unused)]
    #[inline(always)]
    pub unsafe fn get_from_fp(fp: *const u8) -> Option<Self> {
        unsafe { StackFrame::from_fp(fp).map(|s| Self(s)) }
    }

    /// Gets the current Stack Trace, unsafe because the StackTrace may be corrupted
    #[inline(always)]
    pub unsafe fn current() -> Self {
        Self(unsafe { StackFrame::get_current() })
    }
}

impl<'a> Display for StackTrace<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe {
            const MAX_TRACES: usize = 16;

            let mut fp = self.0;
            writeln!(f, "\x1B[34mStack trace:")?;
            for _ in 0..MAX_TRACES {
                let address = fp.call_ptr();

                let name = KERNEL_SYMS
                    .map(|syms| syms.get_name_of(address as u64))
                    .flatten();
                let name = name.unwrap_or("??");
                writeln!(f, "  {:?} <{:#}>", address, name,)?;

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
