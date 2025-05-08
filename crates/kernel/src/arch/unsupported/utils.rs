use serde::Serialize;
use spin::Lazy;

#[derive(Serialize, Debug)]
pub struct CpuInfo {
    vendor_id: heapless::String<12>,
    model: heapless::String<48>,
    core_count: u8,
}

impl CpuInfo {
    pub fn fetch() -> Self {
        let vendor_id = heapless::String::new();
        let model = heapless::String::new();

        Self {
            vendor_id,
            model,
            core_count: -1i8 as u8,
        }
    }
}

pub static CPU_INFO: Lazy<CpuInfo> = Lazy::new(CpuInfo::fetch);

#[inline(always)]
// actually used in macros but rust thinks it is unused for some reason
#[allow(unused)]
/// Returns the number of milliseconds since the CPU was started
pub fn time() -> u64 {
    0
}
