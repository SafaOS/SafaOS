use core::{arch::asm, fmt::Debug};

use bitfield_struct::bitfield;
use int_enum::IntEnum;

/// A unique ID for each CPU
///
/// in x86_64 that is the LAPIC ID
/// while in aarch64(current) that is the whole affinity clustures as indicated by MPIDR_EL1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchCpuID {
    aff0: u8,
    aff1: u8,
    aff2: u8,
    aff3: u8,
}

impl ArchCpuID {
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

    /// Gets the current [`ArchCpuID`]
    pub fn get() -> Self {
        MPIDR::read().cpuid()
    }

    pub unsafe fn from_raw(raw: u64) -> Self {
        MPIDR::from_bits(raw).cpuid()
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
    pub const fn cpuid(&self) -> ArchCpuID {
        ArchCpuID {
            aff0: self.aff0(),
            aff1: self.aff1(),
            aff2: self.aff2(),
            aff3: self.aff3(),
        }
    }

    pub fn read() -> Self {
        let raw: usize;
        unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) raw, options(nostack, nomem)) }

        unsafe { core::mem::transmute(raw) }
    }
}

impl core::fmt::Display for ArchCpuID {
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

const MAIR_IIII_MASK: u8 = 0b00001111;
const MAIR_OOOO_MASK: u8 = 0b11110000;
const MAIR_DEVICE_MASK: u8 = 0b00001100;

#[derive(Debug, Clone, Copy, IntEnum)]
#[repr(u8)]
/// Device memory is encoded as 0b0000xx00 where xx is whatever in this.
pub enum DeviceMemCfg {
    /// No gathering no reordering and no early write acknowledgement.
    NGnRnE = 0b00,
    /// No gathering, no reordering, but allow early write acknowledgement.
    NGnRE = 0b01,
    /// No gathering, but allow reordering, and early write acknowledgement.
    NGRE = 0b10,
    /// Allow gathering, reordering and early write acknowledgement.
    GRE = 0b11,
}

#[derive(Debug, Clone, Copy, IntEnum)]
#[repr(u8)]
/// Normal memory is encoded as 0bxxxxyyyy
/// where x is outer level and y is the inner level, currently we have inner == outer.
///
/// R is the inner/outer read-allocate policy and W is the inner/outer write-allocate policy.
/// I don't know what these means but you want them both on.
pub enum NormalMemCfg {
    /// no caching
    NonCacheable = 0b0100,

    /// write-though normal memory without transient hint bit set.
    WriteThroughRW = 0b1011,
    /// write-back normal memory without transient hint bit set.
    WriteBackRW = 0b1111,
    /// write-though normal memory with transient hint bit set.
    WriteThroughRWT = 0b0011,
    /// write-back normal memory with transient hint bit set.
    WriteBackRWT = 0b0111,

    /// write-though normal memory without transient hint bit set.
    WriteThroughR = 0b1010,
    /// write-back normal memory without transient hint bit set.
    WriteBackR = 0b1110,
    /// write-though normal memory with transient hint bit set.
    WriteThroughRT = 0b0010,
    /// write-back normal memory with transient hint bit set.
    WriteBackRT = 0b0110,

    /// write-though normal memory without transient hint bit set.
    WriteThroughW = 0b1001,
    /// write-back normal memory without transient hint bit set.
    WriteBackW = 0b1101,
    /// write-though normal memory with transient hint bit set.
    WriteThroughWT = 0b0001,
    /// write-back normal memory with transient hint bit set.
    WriteBackWT = 0b0101,

    /// write-though normal memory without transient hint bit set.
    ///
    /// This should be an invalid state
    WriteThroughNone = 0b1000,
    /// write-back normal memory without transient hint bit set.
    ///
    /// This should be an invalid state,
    WriteBackNone = 0b1100,
    /// write-though normal memory with transient hint bit set.
    ///
    /// This should be unreachable.
    WriteThroughNoneT = 0b0000,
}

#[derive(Debug, Clone, Copy)]
pub enum MAIRAttr {
    Device(DeviceMemCfg),
    Normal {
        outer: NormalMemCfg,
        inner: NormalMemCfg,
    },
    Other(u8),
}

impl MAIRAttr {
    pub const fn new_normal(cfg: NormalMemCfg) -> Self {
        Self::Normal {
            outer: cfg,
            inner: cfg,
        }
    }

    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Device(DeviceMemCfg::NGnRnE),
            x if x & MAIR_OOOO_MASK == 0 && x & (1 << 0) == 0 => {
                MAIRAttr::Device(DeviceMemCfg::try_from((x >> 2) & MAIR_DEVICE_MASK).unwrap())
            }
            x if x & MAIR_OOOO_MASK != 0 && x & MAIR_IIII_MASK != 0 => MAIRAttr::Normal {
                outer: NormalMemCfg::try_from((x & MAIR_OOOO_MASK) >> 4).unwrap(),
                inner: NormalMemCfg::try_from(x & MAIR_IIII_MASK).unwrap(),
            },
            x => Self::Other(x),
        }
    }

    pub const fn to_raw(self) -> u8 {
        match self {
            Self::Device(d) => (d as u8) << 2,
            Self::Normal { outer, inner } => inner as u8 | ((outer as u8) << 4),
            Self::Other(o) => o,
        }
    }
}

pub const DEVICE_UNCACHEABLE_MAIR_IDX: u8 = 2;
pub const FRAMEBUFFER_CACHED_MAIR_IDX: u8 = 1;

/// System MAIR Register (memory cache configuration)
pub const SYS_MAIR: MAIR = {
    let mut this = MAIR::new();
    this.set(0, MAIRAttr::new_normal(NormalMemCfg::WriteBackRW));
    this.set(
        DEVICE_UNCACHEABLE_MAIR_IDX as usize,
        MAIRAttr::Device(DeviceMemCfg::NGnRnE),
    );
    this.set(
        FRAMEBUFFER_CACHED_MAIR_IDX as usize,
        MAIRAttr::new_normal(NormalMemCfg::NonCacheable),
    );
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

    /// Sets an attr at a given index.
    pub const fn set(&mut self, index: usize, attr: MAIRAttr) {
        let raw = attr.to_raw();
        self.attributes[index] = raw;
    }

    /// Sets MAIR_EL1 register to `self`
    pub unsafe fn sync(self) {
        let crr_mair: u64;
        unsafe {
            asm!("mrs {}, mair_el1", out(reg) crr_mair);
        }

        let crr_mair: MAIR = unsafe { core::mem::transmute(crr_mair) };
        crate::logging::sprintln!("MAIR was {crr_mair:x?}\nnow {self:x?}");

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
use crate::percpu::CpuLocal;
#[inline(always)]
pub fn cpu_local() -> &'static CpuLocal {
    let ptr: *mut CpuLocal;
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el1", out(reg) ptr, options(nostack, nomem, preserves_flags))
    }
    unsafe { &*ptr }
}

#[inline(always)]
pub unsafe fn set_cpu_local(local: &'static CpuLocal) {
    unsafe { core::arch::asm!("msr tpidr_el1, {}", in(reg) local, options(nostack, nomem)) }
}
