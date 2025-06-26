use crate::{
    threading::{expose::getinfo, Pid},
    utils::alloc::PageString,
};

use super::GenericProcFSFile as ProcFSFile;

pub struct TaskInfoFile;

impl TaskInfoFile {
    pub const fn new(pid: Pid) -> ProcFSFile {
        ProcFSFile::new("info", pid as usize, Self::fetch)
    }

    pub fn fetch(file: &mut ProcFSFile) -> Option<PageString> {
        let mut str = PageString::with_capacity(1024);
        let task_info = getinfo(file.id as Pid).unwrap();

        serde_json::to_writer_pretty(&mut str, &task_info)
            .ok()
            .map(|()| str)
    }
}
