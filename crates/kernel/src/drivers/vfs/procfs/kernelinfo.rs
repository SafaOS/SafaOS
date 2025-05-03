use serde::Serialize;

use crate::{utils::alloc::PageString, KERNEL_CODE_NAME, KERNEL_CODE_VERSION};

use super::ProcFSFile;

const COMPILE_TIME: &str = compile_time::time_str!();
const COMPILE_DATE: &str = compile_time::date_str!();

#[derive(Serialize)]
struct KernelInfo {
    name: &'static str,
    version: &'static str,
    /// the date the kernel was compiled as year-month-day
    compile_date: &'static str,
    /// the time the kernel was compiled as hour:minute:second
    compile_time: &'static str,
    uptime: u64,
}

impl KernelInfo {
    pub fn fetch() -> Self {
        Self {
            name: KERNEL_CODE_NAME,
            version: KERNEL_CODE_VERSION,
            compile_date: COMPILE_DATE,
            compile_time: COMPILE_TIME,
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
