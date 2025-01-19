use alloc::string::String;
use serde::Serialize;

use crate::{KERNEL_CODE_NAME, KERNEL_CODE_VERSION};

use super::ProcFSFile;

#[derive(Serialize)]
struct KernelInfo {
    name: &'static str,
    version: &'static str,
}

impl KernelInfo {
    pub fn fetch() -> Self {
        Self {
            name: KERNEL_CODE_NAME,
            version: KERNEL_CODE_VERSION,
        }
    }
}
pub struct KernelInfoFile;

impl KernelInfoFile {
    pub fn new() -> ProcFSFile {
        ProcFSFile::new_static("kernelinfo", 0, Self::fetch)
    }

    fn fetch(_: &mut ProcFSFile) -> Option<String> {
        let kernel_info = KernelInfo::fetch();
        serde_json::to_string_pretty(&kernel_info).ok()
    }
}
