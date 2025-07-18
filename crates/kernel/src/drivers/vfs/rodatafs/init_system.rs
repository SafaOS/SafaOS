//! This module contains the boot-time implementiation of the initialization of the procfs.

use super::{eve_journal::EVEJournal, usbinfo::USBInfoFile};
use crate::threading::Pid;

use super::{
    GenericRodFSFile, cpuinfo::CpuInfoFile, kernelinfo::KernelInfoFile, meminfo::MemInfoFile,
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

pub const fn process_init_system(process_pid: Pid) -> [InitStateItem; 1] {
    [InitStateItem::File(super::processes::ProcessInfoFile::new(
        process_pid,
    ))]
}
