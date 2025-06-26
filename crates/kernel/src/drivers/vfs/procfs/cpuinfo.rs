use crate::{arch::utils::CPU_INFO, utils::alloc::PageString};

use super::GenericProcFSFile as ProcFSFile;

pub struct CpuInfoFile;
impl CpuInfoFile {
    pub const fn new() -> ProcFSFile {
        ProcFSFile::new_static("cpuinfo", 0, Self::fetch)
    }

    pub fn fetch(_: &mut ProcFSFile) -> Option<PageString> {
        let mut page_string = PageString::with_capacity(1024);
        let cpu_info = &*CPU_INFO;

        serde_json::to_writer_pretty(&mut page_string, cpu_info)
            .ok()
            .map(|()| page_string)
    }
}
