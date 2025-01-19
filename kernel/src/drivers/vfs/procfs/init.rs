//! This module contains the boot-time implementiation of the initialization of the procfs.

use super::{cpuinfo::CpuInfoFile, meminfo::MemInfoFile, ProcFSFile};

pub enum InitStateItem {
    File(ProcFSFile),
}

pub fn get_init_state() -> [InitStateItem; 2] {
    [
        InitStateItem::File(CpuInfoFile::new()),
        InitStateItem::File(MemInfoFile::new()),
    ]
}
