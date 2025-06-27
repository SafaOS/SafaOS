use crate::{
    drivers::vfs::rodatafs::GenericRodFSFile,
    threading::{expose::getinfo, Pid},
    utils::alloc::PageString,
};

pub struct TaskInfoFile;

impl TaskInfoFile {
    pub const fn new(pid: Pid) -> GenericRodFSFile {
        GenericRodFSFile::new("info", pid as usize, Self::fetch)
    }

    pub fn fetch(file: &mut GenericRodFSFile) -> Option<PageString> {
        let mut str = PageString::with_capacity(1024);
        let task_info = getinfo(file.id as Pid).unwrap();

        serde_json::to_writer_pretty(&mut str, &task_info)
            .ok()
            .map(|()| str)
    }
}
