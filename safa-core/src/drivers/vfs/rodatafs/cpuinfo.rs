use crate::{
    arch::utils::CPU_INFO, drivers::vfs::rodatafs::GenericRodFSFile, utils::alloc::PageString,
};

pub struct CpuInfoFile;
impl CpuInfoFile {
    pub const fn new() -> GenericRodFSFile {
        GenericRodFSFile::new_static("cpuinfo", 0, Self::fetch)
    }

    pub fn fetch(_: &mut GenericRodFSFile) -> Option<PageString> {
        let mut page_string = PageString::with_capacity(1024);
        let cpu_info = &*CPU_INFO;

        serde_json::to_writer_pretty(&mut page_string, cpu_info)
            .ok()
            .map(|()| page_string)
    }
}
