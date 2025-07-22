use crate::{
    drivers::vfs::rodatafs::GenericRodFSFile, scheduler::expose::getinfo, utils::alloc::PageString,
};

use crate::process::Pid;
pub struct ProcessInfoFile;

impl ProcessInfoFile {
    pub const fn new(pid: Pid) -> GenericRodFSFile {
        GenericRodFSFile::new("info", pid as usize, Self::fetch)
    }

    pub fn fetch(file: &mut GenericRodFSFile) -> Option<PageString> {
        let mut str = PageString::with_capacity(1024);
        let process_info = getinfo(file.id as Pid).unwrap();

        serde_json::to_writer_pretty(&mut str, &process_info)
            .ok()
            .map(|()| str)
    }
}
