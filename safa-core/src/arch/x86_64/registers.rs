use bitflags::bitflags;
use core::{arch::asm, fmt::Display};

use crate::{
    VirtAddr,
    arch::x86_64::interrupts::apic::APIC,
    memory::paging::{Page, PageTable},
};

bitflags! {
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
            asm!(
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
    pub fn get() -> Self {
        // If there is no APIC it means there is no CPUs yet except for the boot cpu
        Self(APIC.get().map(|a| a.lapic_id()).unwrap_or(0))
    }

    /// Create a new CPUID from a limine CPU
    pub fn from_cpu(cpu: &limine::mp::Cpu) -> Self {
        Self(cpu.lapic_id as u8)
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
    unsafe { wrmsr(0xC0000100, value.into_raw() as u64) }
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

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StackFrame {
    prev: *mut StackFrame,
    return_addr: *mut u8,
}

impl StackFrame {
    pub unsafe fn from_fp<'a>(ptr: *const u8) -> Option<&'a Self> {
        unsafe {
            let fp: *mut Self = ptr.cast::<Self>().cast_mut();

            if PageTable::current()
                .get_frame_of(Page::containing(VirtAddr::from_ptr(fp)))
                .is_none()
                || !fp.is_aligned()
            {
                return None;
            } else {
                Some(&*fp)
            }
        }
    }
    /// Gets the current Frame Pointer from the fp register
    pub unsafe fn get_current<'a>() -> &'a Self {
        unsafe {
            let fp: *mut Self;
            asm!("mov {}, rbp", out(reg) fp);
            &*fp
        }
    }

    /// Gets the return address from the Frame
    pub fn return_ptr(&self) -> *mut u8 {
        self.return_addr
    }

    /// Gets the previous Frame Pointer from this one
    pub unsafe fn prev(&self) -> Option<&Self> {
        let prev = self.prev;

        if prev.is_null() || !prev.is_aligned() || (prev as usize) < 0x1000 {
            return None;
        }

        if unsafe {
            PageTable::current()
                .get_frame_of(Page::containing(VirtAddr::from_ptr(prev)))
                .is_none()
        } {
            return None;
        }
        unsafe { Some(&*prev) }
    }
}
