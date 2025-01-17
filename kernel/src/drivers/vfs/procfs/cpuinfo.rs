use alloc::string::String;

use crate::arch::utils::CpuInfo;

use super::ProcFSFile;

#[derive(Clone)]
pub struct CpuInfoWrapper(Option<String>);

impl CpuInfoWrapper {
    pub const fn new() -> Self {
        Self(None)
    }
}

impl ProcFSFile for CpuInfoWrapper {
    fn name(&self) -> &'static str {
        "cpuinfo"
    }

    fn close(&mut self) {
        self.0 = None;
    }

    fn refresh(&mut self) {
        self.0 = serde_json::to_string_pretty(&CpuInfo::fetch()).ok();
    }

    fn try_get_data(&self) -> Option<&str> {
        self.0.as_deref()
    }
}
