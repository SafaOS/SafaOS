use alloc::string::String;
use serde::Serialize;

use crate::memory::frame_allocator::{mapped_frames, usable_frames};

use super::ProcFSFile;

#[derive(Clone, Serialize)]
pub struct MemInfo {
    total: usize,
    free: usize,
    used: usize,
    // TODO: IMPLEMENT
    current_process_used: usize,
}

impl MemInfo {
    pub fn fetch() -> Self {
        let total = usable_frames() * 4096;
        let used = mapped_frames() * 4096;
        let free = total - used;

        let current_process_used = 0;

        Self {
            total,
            free,
            used,
            current_process_used,
        }
    }
}

pub struct MemInfoFile;

impl MemInfoFile {
    pub fn new() -> ProcFSFile {
        ProcFSFile::new("meminfo", 0, Self::fetch)
    }

    pub fn fetch(_: &mut ProcFSFile) -> Option<String> {
        let mem_info = MemInfo::fetch();
        serde_json::to_string_pretty(&mem_info).ok()
    }
}
