use alloc::string::String;

use crate::threading::{expose::getinfo, Pid};

use super::ProcFSFile;

pub struct TaskInfoFile;

impl TaskInfoFile {
    pub fn new(pid: Pid) -> ProcFSFile {
        ProcFSFile::new("info", pid, Self::fetch)
    }

    pub fn fetch(file: &mut ProcFSFile) -> Option<String> {
        let task_info = getinfo(file.id).unwrap();
        serde_json::to_string_pretty(&task_info).ok()
    }
}
