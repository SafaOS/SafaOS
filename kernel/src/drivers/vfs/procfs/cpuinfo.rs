use alloc::string::String;

use crate::arch::utils::CPU_INFO;

use super::ProcFSFile;

#[derive(Clone)]
pub struct CpuInfoWrapper(String);

impl CpuInfoWrapper {
    pub fn new() -> Self {
        Self(serde_json::to_string_pretty(&*CPU_INFO).unwrap())
    }
}

impl ProcFSFile for CpuInfoWrapper {
    fn name(&self) -> &'static str {
        "cpuinfo"
    }

    fn close(&mut self) {}

    fn refresh(&mut self) {}

    fn try_get_data(&self) -> Option<&str> {
        Some(self.0.as_str())
    }
}
