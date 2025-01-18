//! This module contains the boot-time implementiation of the initialization of the procfs.

use alloc::boxed::Box;
use spin::Lazy;

use crate::drivers::vfs::procfs::cpuinfo;

use super::{cpuinfo::CpuInfoWrapper, ProcFSFile};

pub trait IntoBoxFile: Send + Sync {
    fn into_box(&self) -> Box<dyn ProcFSFile>;
}

pub enum InitStateItem<'a> {
    File(&'a dyn IntoBoxFile),
    #[allow(unused)]
    Directory(&'a str, &'a [Self]),
}

impl<T: ProcFSFile + Clone + 'static> IntoBoxFile for T {
    fn into_box(&self) -> Box<dyn ProcFSFile> {
        Box::new(Clone::clone(self))
    }
}

static CPU_INFO: Lazy<CpuInfoWrapper> = Lazy::new(cpuinfo::CpuInfoWrapper::new);

pub static INIT_STATE: Lazy<[InitStateItem<'static>; 1]> =
    Lazy::new(|| [InitStateItem::File(&*CPU_INFO)]);
