use crate::{
    threading::{expose::getinfo, Pid},
    utils::alloc::PageString,
};

use super::ProcFSFile;

pub struct TaskInfoFile;

impl TaskInfoFile {
    pub fn new(pid: Pid) -> ProcFSFile {
        ProcFSFile::new("info", pid, Self::fetch)
    }

    pub fn fetch(file: &mut ProcFSFile) -> Option<PageString> {
        let mut str = PageString::with_capacity(1024);
        let task_info = getinfo(file.id).unwrap();

        serde_json::to_writer_pretty(&mut str, &task_info)
            .ok()
            .map(|()| str)
    }
}
