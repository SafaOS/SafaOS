use core::{
    fmt::{Debug, Display, LowerHex, UpperHex},
    ops::Deref,
};

use int_enum::IntEnum;

#[derive(Clone, Copy)]
pub struct GeneralPurpose(u64);
macro_rules! impl_common {
    ($mod: path) => {
        impl LowerHex for $mod {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{:#x}", self.0)
            }
        }

        impl UpperHex for $mod {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{:#X}", self.0)
            }
        }

        impl Debug for $mod {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{:#x}", self)
            }
        }

        impl Deref for $mod {
            type Target = u64;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

impl_common!(GeneralPurpose);

#[derive(Clone, Copy, Debug, IntEnum, PartialEq, Eq)]
#[repr(u8)]
pub enum ExcClass {
    Unknown = 0b000000,
    TrappedWF = 0b000001,
    IllegalExecution = 0b001110,
    SysCall = 0b010101,
    InstrAbortLower = 0b100000,
    InstrAbort = 0b100001,
    InstrAlignmentFault = 0b100010,
    DataAbortLower = 0b100100,
    DataAbort = 0b100101,
    StackAlignmentFault = 0b100110,
    FloatingPoint = 0b101100,
}

#[derive(Copy, Clone)]
pub struct Esr(u64);
impl_common!(Esr);

impl Esr {
    #[inline(always)]
    const fn class_raw(&self) -> u8 {
        let value = (self.0 >> 26) & ((1 << 6) - 1);
        value as u8
    }

    #[inline(always)]
    pub fn class(&self) -> ExcClass {
        ExcClass::try_from(self.class_raw()).unwrap_or(ExcClass::Unknown)
    }
}

impl Display for Esr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "ESR_EL1: {:?}", self)?;

        let class = self.class();
        write!(f, "Exception Class: {:#06b} {:?}", self.class_raw(), class)?;

        if class == ExcClass::DataAbort || class == ExcClass::DataAbortLower {
            let cause = (self.0 >> 2) & 0x3;
            let level = (self.0) & 0x3;
            let fnv = ((self.0 >> 10) & 1) == 1;

            let cause = match cause {
                0 => Some("Address Size Fault"),
                1 => Some("Translation Fault"),
                2 => Some("Access Flag Fault"),
                3 => Some("Permission Fault"),
                _ => None,
            };

            if let Some(cause) = cause {
                write!(f, " ({cause})")?;
            }

            if level <= 3 {
                write!(f, " (L{level})")?;
            }

            let fv = if fnv { "not valid" } else { "valid" };
            write!(f, " (FAR {fv})")?;
        }
        Ok(())
    }
}
