use core::{arch::asm, fmt::Display};

use crate::{
    misc::{PhysAddr, VirtAddr},
    percpu::CpuLocal,
};

bitflags::bitflags! {
    #[derive(Default, Debug, Clone, Copy)]
    #[repr(C)]
    pub struct RFLAGS: u64 {
        const ID = 1 << 21;
        const VIRTUAL_INTERRUPT_PENDING = 1 << 20;
        const VIRTUAL_INTERRUPT = 1 << 19;
        const ALIGNMENT_CHECK = 1 << 18;
        const VIRTUAL_8086_MODE = 1 << 17;

        const RESUME_FLAG = 1 << 16;
        const NESTED_TASK = 1 << 14;

        const IOPL_HIGH = 1 << 13;
        const IOPL_LOW = 1 << 12;

        const OVERFLOW_FLAG = 1 << 11;
        const DIRECTION_FLAG = 1 << 10;

        const INTERRUPT_FLAG = 1 << 9;
        const TRAP_FLAG = 1 << 8;

        const SIGN_FLAG = 1 << 7;
        const ZERO_FLAG = 1 << 6;
        const AUXILIARY_CARRY_FLAG = 1 << 4;

        const PARITY_FLAG = 1 << 2;
        const CARRY_FLAG = 1;
    }
}

impl RFLAGS {
    #[inline]
    pub const fn interrupts_enabled(&self) -> bool {
        self.contains(Self::INTERRUPT_FLAG)
    }

    pub fn read() -> Self {
        let result: u64;
        unsafe {
            core::arch::asm!(
                "pushfq; pop {}",
                out(reg) result, options(nomem, preserves_flags)
            );
            Self::from_bits_retain(result)
        }
    }
}

/// A unique ID for each CPU
///
/// in x86_64(current) that is the LAPIC ID
/// while in aarch64 that is the whole affinity clustures as indicated by MPIDR_EL1
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArchCpuID(u8);

impl ArchCpuID {
    pub(crate) fn get() -> Self {
        // If there is no APIC it means there is no CPUs yet except for the boot cpu
        // TODO: Implement
        Self(0)
    }

    pub unsafe fn from_raw(id: u64) -> Self {
        Self(id as u8)
    }

    pub(super) const fn lapic_id(self) -> u8 {
        self.0
    }
}

impl Display for ArchCpuID {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn rdmsr(msr: u32) -> usize {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr, out("eax") low, out("edx") high
        );
    }

    (high as usize) << 32 | (low as usize)
}

#[inline(always)]
pub fn rdfsbase() -> VirtAddr {
    VirtAddr::from(rdmsr(0xC0000100))
}

#[inline(always)]
pub unsafe fn wrfsbase(value: VirtAddr) {
    unsafe { wrmsr(0xC0000100, value.raw() as u64) }
}

pub unsafe fn wrmsr(msr: u32, value: u64) {
    let (low, high) = (value as u32, (value >> 32) as u32);
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr, in("eax") low, in("edx") high, options(nostack, preserves_flags)
        );
    }
}

#[inline(always)]
pub unsafe fn set_gs(value: VirtAddr) {
    unsafe {
        wrmsr(0xC0000101, value.raw() as u64);
        wrmsr(0xC0000102, value.raw() as u64);
        core::arch::asm!("swapgs");
    }
}

#[inline(always)]
pub fn cpu_local() -> &'static CpuLocal {
    let v: usize;
    unsafe {
        core::arch::asm!("mov {}, gs:0", out(reg) v, options(nostack, readonly, preserves_flags))
    };

    unsafe { &*(v as *const CpuLocal) }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InterruptFrame {
    rip: VirtAddr,
    cs: u64,
    rflags: RFLAGS,
    rsp: VirtAddr,
    ss: u64,
}

#[derive(Debug, Clone, Copy)]
#[repr(C, align(16))]
pub struct CapturedCpuStatus {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rdi: u64,
    rsi: u64,

    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,

    cr3: PhysAddr,
    rbp: u64,
    error_code: u64,
    frame: InterruptFrame,
}

impl CapturedCpuStatus {
    pub const fn rip(&self) -> VirtAddr {
        self.frame.rip
    }

    pub const fn rsp(&self) -> VirtAddr {
        self.frame.rsp
    }

    pub const fn rflags(&self) -> RFLAGS {
        self.frame.rflags
    }

    pub const fn error_code(&self) -> u64 {
        self.error_code
    }
}

impl Display for CapturedCpuStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Saved general purpose registers:")?;
        macro_rules! reg {
            ($name:ident) => {
                write!(f, "{:<3}: {:#018x?}    ", stringify!($name), self.$name)?;
            };
        }

        reg!(rax);
        reg!(rbx);
        reg!(rcx);

        writeln!(f)?;

        reg!(rdx);
        reg!(rdi);
        reg!(rsi);

        writeln!(f)?;

        reg!(r8);
        reg!(r9);
        reg!(r10);

        writeln!(f)?;
        reg!(r11);
        reg!(r12);
        reg!(r13);

        writeln!(f)?;
        reg!(r14);
        reg!(r15);

        write!(f, "\n\n")?;
        writeln!(f)?;
        reg!(rbp);
        writeln!(f)?;
        reg!(cr3);
        writeln!(f)?;

        writeln!(
            f,
            "rsp: {:?}, at {:?} <{}>",
            self.rsp(),
            self.rip(),
            "UNNAMED"
        )?;
        writeln!(f, "rflags: {:#?}", self.rflags())?;
        Ok(())
    }
}
