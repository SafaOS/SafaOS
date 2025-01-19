use alloc::string::String;

use crate::arch::utils::CPU_INFO;

use super::ProcFSFile;

pub struct CpuInfoFile;
impl CpuInfoFile {
    pub fn new() -> ProcFSFile {
        ProcFSFile::new_static("cpuinfo", 0, Self::fetch)
    }

    pub fn fetch(_: &mut ProcFSFile) -> Option<String> {
        let cpu_info = &*CPU_INFO;
        serde_json::to_string_pretty(cpu_info).ok()
    }
}
