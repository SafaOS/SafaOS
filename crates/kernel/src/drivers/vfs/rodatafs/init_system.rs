//! This module contains the boot-time implementiation of the initialization of the procfs.

use super::{eve_journal::EVEJournal, usbinfo::USBInfoFile};
use crate::threading::Pid;

use super::{
    cpuinfo::CpuInfoFile, kernelinfo::KernelInfoFile, meminfo::MemInfoFile, GenericRodFSFile,
};

pub enum InitStateItem {
    File(GenericRodFSFile),
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
