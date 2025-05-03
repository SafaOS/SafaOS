//! This module contains the boot-time implementiation of the initialization of the procfs.

use super::{cpuinfo::CpuInfoFile, kernelinfo::KernelInfoFile, meminfo::MemInfoFile, ProcFSFile};

pub enum InitStateItem {
    File(ProcFSFile),
}

pub fn get_init_state() -> [InitStateItem; 3] {
    [
        InitStateItem::File(CpuInfoFile::new()),
        InitStateItem::File(MemInfoFile::new()),
        InitStateItem::File(KernelInfoFile::new()),
    ]
}
