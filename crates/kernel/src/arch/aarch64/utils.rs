use core::arch::asm;

use super::registers::MIDR;
use core::fmt::Write;
use serde::Serialize;
use spin::Lazy;

#[derive(Serialize, Debug)]
pub struct CpuInfo {
    vendor_id: heapless::String<12>,
    model: heapless::String<48>,
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
        let partnum = midr.partnum();

        write!(vendor_id, "{:?}", implementer).expect("vendor id too long");

        let mut model = heapless::String::new();
        if let Some(model_id) = implementer.cpu_model(partnum) {
            write!(model, "{}", model_id).expect("model name too long");
        } else {
            write!(model, "{:#x}", partnum).expect("model number too long");
        }

        Self {
            vendor_id,
            model,
            arch: "aarch64",
            core_count: Self::fetch_core_count(),
        }
    }
}

pub static CPU_INFO: Lazy<CpuInfo> = Lazy::new(CpuInfo::fetch);

#[inline(always)]
/// Returns the number of milliseconds since the CPU was started
pub fn time() -> u64 {
    let count: u64;
    let freq: u64;
    unsafe {
        core::arch::asm!(
            "isb",
            "mrs {cnt}, cntpct_el0",
            "mrs {frq}, cntfrq_el0",
            cnt = out(reg) count,
            frq = out(reg) freq,
        );
    }
    count / (freq / 1000)
}
