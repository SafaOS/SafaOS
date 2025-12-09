use core::{arch::asm, num::NonZero};

use super::registers::MIDR;
use core::fmt::Write;
use serde::Serialize;
use spin::Lazy;

#[derive(Serialize, Debug)]
pub struct CpuInfo {
    vendor_id: heapless::String<12>,
    model: &'static str,
    arch: &'static str,
    core_count: u8,
}

impl CpuInfo {
    fn fetch_core_count() -> u8 {
        let mpidr_el1: usize;
        unsafe {
            asm!("mrs {}, mpidr_el1", out(reg) mpidr_el1);
        }
        ((mpidr_el1 & 0x3) as u8) + 1
    }
    pub fn fetch() -> Self {
        let midr = MIDR::read();
        let mut vendor_id = heapless::String::new();

        let implementer = midr.implementer();
        write!(vendor_id, "{:?}", implementer).expect("vendor id too long");

        Self {
            vendor_id,
            model: unsafe { &*super::cpu::MODEL.get() },
            arch: "aarch64",
            core_count: Self::fetch_core_count(),
        }
    }
}

pub static CPU_INFO: Lazy<CpuInfo> = Lazy::new(CpuInfo::fetch);

#[inline]
/// Returns the frequency of the CPU
pub fn cpu_timer_freq_mhz() -> NonZero<u64> {
    let freq: u64;
    unsafe {
        core::arch::asm!(
            "mrs {frq}, cntfrq_el0",
            frq = out(reg) freq,
        );
    }

    unsafe { NonZero::new_unchecked(freq / 1_000_000) }
}

#[inline(always)]
#[allow(unused)]
/// Returns the number of CPU cycles since the CPU was started
pub fn cpu_cycles() -> u64 {
    let count: u64;
    unsafe {
        core::arch::asm!(
            "isb",
            "mrs {cnt}, cntpct_el0",
            cnt = out(reg) count,
        );
    }
    count
}
