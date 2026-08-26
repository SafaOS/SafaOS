use bitfield_struct::bitfield;

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

use crate::percpu::CpuLocal;
#[inline(always)]
pub fn cpu_local() -> &'static CpuLocal {
    let ptr: *mut CpuLocal;
    unsafe { core::arch::asm!("mrs {}, tpidr_el1", out(reg) ptr, options(nostack, nomem)) }
    unsafe { &*ptr }
}

#[inline(always)]
pub unsafe fn set_cpu_local(local: &'static CpuLocal) {
    unsafe { core::arch::asm!("msr tpidr_el1, {}", in(reg) local, options(nostack, nomem)) }
}
