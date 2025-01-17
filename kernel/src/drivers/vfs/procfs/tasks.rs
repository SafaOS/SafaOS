use alloc::string::String;

use crate::threading::{expose::getinfo, Pid};

use super::ProcFSFile;

pub struct TaskInfoFile {
    pid: Pid,
    data: Option<String>,
}

impl TaskInfoFile {
    pub fn new(pid: Pid) -> Self {
        Self { pid, data: None }
    }
}

impl ProcFSFile for TaskInfoFile {
    fn name(&self) -> &'static str {
        "info"
    }

    fn try_get_data(&self) -> Option<&str> {
        self.data.as_deref()
    }

    fn refresh(&mut self) {
        let task_info = getinfo(self.pid).unwrap();
        self.data = serde_json::to_string_pretty(&task_info).ok();
    }

    fn close(&mut self) {
        self.data = None;
    }
}
