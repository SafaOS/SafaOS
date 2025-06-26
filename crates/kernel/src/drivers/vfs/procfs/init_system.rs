//! This module contains the boot-time implementiation of the initialization of the procfs.

use crate::{
    drivers::vfs::procfs::{eve_journal::EVEJournal, usbinfo::USBInfoFile},
    threading::Pid,
};

use super::{
    cpuinfo::CpuInfoFile, kernelinfo::KernelInfoFile, meminfo::MemInfoFile, GenericProcFSFile,
};

pub enum InitStateItem {
    File(GenericProcFSFile),
}

pub const fn get_init_state() -> [InitStateItem; 5] {
    [
        InitStateItem::File(CpuInfoFile::new()),
        InitStateItem::File(MemInfoFile::new()),
        InitStateItem::File(KernelInfoFile::new()),
        InitStateItem::File(USBInfoFile::new()),
        InitStateItem::File(EVEJournal::new()),
    ]
}

pub const fn task_init_system(task_pid: Pid) -> [InitStateItem; 1] {
    [InitStateItem::File(super::tasks::TaskInfoFile::new(
        task_pid,
    ))]
}
