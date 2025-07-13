use core::{
    arch::asm,
    fmt::{Debug, Display, LowerHex, UpperHex},
    ops::Deref,
};

use bitfield_struct::bitfield;
use bitflags::bitflags;
use int_enum::IntEnum;

#[derive(Clone, Copy, Default)]
#[repr(transparent)]
pub(super) struct Reg(pub u64);

macro_rules! impl_common {
    ($mod: path) => {
        impl LowerHex for $mod {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{:#016x}", self.0)
            }
        }

        impl UpperHex for $mod {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{:#016X}", self.0)
            }
        }

        impl Debug for $mod {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "Reg({:#x})", self)
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

impl_common!(Reg);

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

#[derive(Copy, Clone, Default)]
#[repr(transparent)]
pub(super) struct Esr(u64);
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

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub(super) struct FramePointer(*mut StackFrame);

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StackFrame {
    prev: FramePointer,
    return_addr: *mut u8,
}

impl StackFrame {
    /// Gets the current Frame Pointer from the fp register
    pub unsafe fn get_current<'a>() -> &'a Self {
        unsafe {
            let fp: *mut Self;
            asm!("mov {}, fp", out(reg) fp);
            &*fp
        }
    }

    /// Gets the return address from the Frame
    pub fn return_ptr(&self) -> *mut u8 {
        self.return_addr
    }

    /// Gets the previous Frame Pointer from this one
    pub unsafe fn prev(&self) -> Option<&Self> {
        let prev = self.prev.0;

        if prev.is_null() || !prev.is_aligned() || (prev as usize) < 0x1000 {
            return None;
        }
        unsafe { Some(&*prev) }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, Default)]
    #[repr(C)]
    pub struct Spsr: u64 {
        const Neg = 1 << 31;
        const Zero = 1 << 30;
        const Carry = 1 << 29;
        const V = 1 << 28;
        const Q = 1 << 27;

        /// Debug interrupt mask
        const D = 1 << 9;
        /// SError exception mask
        const A = 1 << 8;
        /// IRQ interrupt mask
        const I = 1 << 7;
        /// FIQ interrupt mask
        const F = 1 << 6;
        const EL1H = 0b0101;
    }
}

#[derive(Debug, Clone, Copy, IntEnum)]
#[repr(u8)]
pub enum MIDRImplementer {
    Unknown = 0x0,
    ArmLimited = 0x41,
    BroadcomCor = 0x42,
    CaviumInc = 0x43,
    DEC = 0x44,
    FujitsuLtd = 0x46,
    Infineon = 0x49,
    Motorola = 0x4D,
    Nividia = 0x4E,
    AMCC = 0x50,
    QualcommInc = 0x51,
    Marvell = 0x56,
    IntelLtd = 0x69,
    AmpereComputing = 0xC0,
}

impl MIDRImplementer {
    /// FIXME: this shouldn't be relayed upon instead we should return raw numbers and let the software figure it out
    /// however i have defined some cpu models for now
    pub fn cpu_model(&self, partnum: u16) -> Option<&'static str> {
        match self {
            Self::ArmLimited => match partnum {
                0xD0A => Some("Cortex-A75"),
                0xD08 => Some("Cortex-A72"),
                0xD03 => Some("Cortex-A53"),
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct MIDR {
    part_and_revision: u16,
    arch_variant: u8,
    implementer: u8,
}

impl MIDR {
    pub fn read() -> Self {
        let midr: u32;
        unsafe {
            asm!("mrs {:x}, midr_el1", out(reg) midr);
        }
        unsafe { core::mem::transmute(midr) }
    }
    pub fn implementer(&self) -> MIDRImplementer {
        MIDRImplementer::try_from(self.implementer).unwrap_or(MIDRImplementer::Unknown)
    }
}

const MAIR_IIII_MASK: u8 = 1 << 0 | 1 << 1 | 1 << 2 | 1 << 3;
const MAIR_OOOO_MASK: u8 = 1 << 4 | 1 << 5 | 1 << 6 | 1 << 7;

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct MAIRDeviceAttr: u8 {
        const NO_XS = 1 << 0;
        const NGNRE = 1 << 2;
        const NGRE = 1 << 3;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct MAIRNormal: u8 {
        // oooo
        const OUTER_WRITE = 1 << 0;
        const OUTER_READ = 1 << 1;
        const OUTER_NON_CACHEABLE = 1 << 2;
        const OUTER_WRITE_BACK_TR =  1 << 2;
        const OUTER_WRITE_THROUGH_NOTR = 1 << 3;
        const OUTER_WRITE_BACK_NOTR = 1 << 2 | 1 << 3;
        // iiii
        const INNER_WRITE = 1 << 4;
        const INNER_READ = 1 << 5;
        const INNER_NON_CACHEABLE = 1 << 6;
        const INNER_WRITE_BACK_TR =  1 << 6;
        const INNER_WRITE_THROUGH_NOTR = 1 << 7;
        const INNER_WRITE_BACK_NOTR = 1 << 6 | 1 << 7;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MAIRAttr {
    Device(MAIRDeviceAttr),
    Normal(MAIRNormal),
    Other(u8),
}

impl MAIRAttr {
    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Device(MAIRDeviceAttr::empty()),
            x if x & MAIR_OOOO_MASK == 0 && x & (1 << 0) == 0 => {
                MAIRAttr::Device(MAIRDeviceAttr::from_bits_retain(x))
            }
            x if x & MAIR_OOOO_MASK != 0 && x & MAIR_IIII_MASK != 0 => {
                MAIRAttr::Normal(MAIRNormal::from_bits_retain(x))
            }
            x => Self::Other(x),
        }
    }

    pub const fn to_raw(self) -> u8 {
        match self {
            Self::Device(d) => d.bits(),
            Self::Normal(n) => n.bits(),
            Self::Other(o) => o,
        }
    }
}

/// System MAIR Register (memory cache configuration)
pub const SYS_MAIR: MAIR = {
    let mut this = MAIR::new();
    // TODO: configure caching better, especially for devices
    this.set(0, MAIRAttr::Normal(MAIRNormal::all()));
    this.set(1, MAIRAttr::Device(MAIRDeviceAttr::empty()));
    this
};

#[derive(Clone, Copy)]
#[repr(C)]
pub struct MAIR {
    attributes: [u8; 8],
}

impl MAIR {
    pub const fn new() -> Self {
        Self { attributes: [0; 8] }
    }

    pub const fn set(&mut self, index: usize, attr: MAIRAttr) {
        let raw = attr.to_raw();
        self.attributes[index] = raw;
    }

    /// Sets MAIR_EL1 register to `self`
    pub unsafe fn sync(self) {
        let mair_el1: u64 = unsafe { core::mem::transmute(self) };
        unsafe {
            asm!("msr mair_el1, {}", in(reg) mair_el1);
        }
    }
}

impl Debug for MAIR {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug_list = f.debug_list();
        for attr in self.attributes {
            debug_list.entry(&MAIRAttr::from_raw(attr));
        }
        debug_list.finish()
    }
}
/// A unique ID for each CPU
///
/// in x86_64 that is the LAPIC ID
/// while in aarch64(current) that is the whole affinity clustures as indicated by MPIDR_EL1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CPUID {
    aff0: u8,
    aff1: u8,
    aff2: u8,
    aff3: u8,
}

impl CPUID {
    pub(super) const fn construct(aff0: u8, aff1: u8, aff2: u8, aff3: u8) -> Self {
        Self {
            aff0,
            aff1,
            aff2,
            aff3,
        }
    }

    pub(super) const fn aff0(&self) -> u8 {
        self.aff0
    }

    pub(super) const fn aff1(&self) -> u8 {
        self.aff1
    }

    pub(super) const fn aff2(&self) -> u8 {
        self.aff2
    }

    pub(super) const fn aff3(&self) -> u8 {
        self.aff3
    }

    /// Gets the current [`CPUID`]
    pub fn get() -> Self {
        MPIDR::read().cpuid()
    }
}
#[bitfield(u64)]
#[derive(PartialEq, Eq)]
pub struct MPIDR {
    /**
    Affinity level 0. The value of the MPIDR.{Aff2, Aff1, Aff0} or MPIDR_EL1.{Aff3, Aff2, Aff1, Aff0} set of fields of each PE must be unique within the system as a whole.

    This field has an IMPLEMENTATION DEFINED value.

    Access to this field is RO.
    */
    #[bits(access = RO)]
    pub aff0: u8,
    #[bits(access = RO)]
    pub aff1: u8,
    #[bits(access = RO)]
    pub aff2: u8,
    /**
    Indicates whether the lowest level of affinity consists of logical PEs that are implemented using an interdependent approach, such as multithreading. See the description of Aff0 for more information about affinity levels.

    The value of this field is an IMPLEMENTATION DEFINED choice of:
    MT	Meaning
    0b0

    Performance of PEs with different affinity level 0 values, and the same values for affinity level 1 and higher, is largely independent.
    0b1

    Performance of PEs with different affinity level 0 values, and the same values for affinity level 1 and higher, is very interdependent.

    This field does not indicate that multithreading is implemented and does not indicate that PEs with different affinity level 0 values, and the same values for affinity level 1 and higher are implemented.
    */
    #[bits(1, access = RO)]
    pub mt: bool,
    #[bits(5)]
    __: (),
    /**
    Indicates a Uniprocessor system, as distinct from PE 0 in a multiprocessor system.

    The value of this field is an IMPLEMENTATION DEFINED choice of:
    U	Meaning
    0b0

    Processor is part of a multiprocessor system.
    0b1

    Processor is part of a uniprocessor system.

    Access to this field is RO.
    */
    #[bits(1, access = RO)]
    pub u: bool,
    #[bits(1)]
    __: (),
    #[bits(access = RO)]
    pub aff3: u8,
    #[bits(24)]
    __: (),
}

impl MPIDR {
    pub const fn cpuid(&self) -> CPUID {
        CPUID {
            aff0: self.aff0(),
            aff1: self.aff1(),
            aff2: self.aff2(),
            aff3: self.aff3(),
        }
    }

    pub fn read() -> Self {
        let raw: usize;
        unsafe { asm!("mrs {}, mpidr_el1", out(reg) raw) }

        unsafe { core::mem::transmute(raw) }
    }
}

impl Display for CPUID {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.aff3(),
            self.aff2(),
            self.aff1(),
            self.aff0()
        )
    }
}
