use serde::Serialize;

use crate::{utils::alloc::PageString, KERNEL_CODE_NAME, KERNEL_CODE_VERSION};

use super::ProcFSFile;

#[derive(Serialize)]
struct KernelInfo {
    name: &'static str,
    version: &'static str,
    uptime: u64,
}

impl KernelInfo {
    pub fn fetch() -> Self {
        Self {
            name: KERNEL_CODE_NAME,
            version: KERNEL_CODE_VERSION,
            uptime: crate::time!(),
        }
    }
}
pub struct KernelInfoFile;

impl KernelInfoFile {
    pub fn new() -> ProcFSFile {
        ProcFSFile::new("kernelinfo", 0, Self::fetch)
    }

    fn fetch(_: &mut ProcFSFile) -> Option<PageString> {
        let mut page_string = PageString::with_capacity(1024);
        let kernel_info = KernelInfo::fetch();
        serde_json::to_writer_pretty(&mut page_string, &kernel_info)
            .ok()
            .map(|()| page_string)
    }
}
