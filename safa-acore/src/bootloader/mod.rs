mod framebuffer;
pub mod limine;
mod memory_map;

pub use framebuffer::*;
pub use memory_map::*;

use core::num::NonZero;

pub use limine as current;

use crate::{
    arch::ArchCpuID,
    misc::{PhysAddr, VirtAddr},
    oninit,
};

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct CpuInfo<'a> {
    inner: &'a current::CpuInfoInner,
}

impl<'a> CpuInfo<'a> {
    const _TRANSMUTE_SAFE: () = assert!(size_of::<&current::CpuInfoInner>() == size_of::<Self>());

    #[inline(always)]
    pub fn cpus() -> Option<impl Iterator<Item = Self>> {
        current::cpus().map(|cpus| cpus.iter().map(|inner| Self { inner }))
    }

    #[inline(always)]
    /// Returns the CPU ID of this CPU.
    pub fn cpu_id(&self) -> ArchCpuID {
        current::cpu_id(self.inner)
    }

    /// Returns the extra argument passed to [`Self::bootstrap`] or None if not supported/not bootstrapped.
    #[inline(always)]
    pub fn extra_argument(&self) -> Option<u64> {
        current::get_extra_arg(self.inner)
    }

    #[must_use = "Should always succeed unless the current bootloader doesn't support it (is not limine)"]
    /// Attempts to make the current cpu execute the function `f` and stores the argument `extra_arg`.
    #[inline(always)]
    pub fn bootstrap(&self, f: unsafe extern "C" fn(Self) -> !, extra_arg: u64) -> bool {
        current::bootstrap(self, unsafe { core::mem::transmute(f) }, extra_arg)
    }
}

/// Returns the frequency of the TSC timer in hertz for the current architecture if
#[inline]
pub fn tsc_freq_hz() -> Option<NonZero<u64>> {
    current::tsc_freq_hz()
}

/// Returns the address of the higher half direct map where P+H => P.
///
/// (H being the HHDM, P being any usable physical memory address, => means mapped to)
#[inline]
pub fn hhdm() -> VirtAddr {
    current::hhdm()
}

#[inline]
/// Returns the physical address of the kernel executable.
pub fn exe_phys() -> PhysAddr {
    current::exe_phys()
}

#[inline]
/// Returns the virtual address of the kernel executable.
pub fn exe_virt() -> VirtAddr {
    current::exe_virt()
}

#[inline]
/// Returns the amount of SMP Cpus.
pub fn cpu_count() -> usize {
    CpuInfo::cpus().map(|c| c.count().max(1)).unwrap_or(1)
}

oninit::define! {
    pub static HHDM: VirtAddr = || hhdm();
}
