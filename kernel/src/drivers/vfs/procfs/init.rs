//! This module contains the boot-time implementiation of the initialization of the procfs.

use alloc::boxed::Box;

use crate::drivers::vfs::procfs::cpuinfo;

use super::{cpuinfo::CpuInfoWrapper, ProcFSFile};

pub trait IntoBoxFile: Send + Sync {
    fn into_box(&self) -> Box<dyn ProcFSFile>;
}

pub enum InitStateItem<'a> {
    File(&'a dyn IntoBoxFile),
    Directory(&'a str, &'a [Self]),
}

impl InitStateItem<'static> {
    #[allow(unused)]
    const fn create_directory(name: &'static str, files: &'static [Self]) -> Self {
        InitStateItem::Directory(name, files)
    }

    const fn create_file<T: IntoBoxFile>(file: &'static T) -> Self {
        InitStateItem::File(file)
    }
}

macro_rules! generate_init_state {
    ($(
        $(dir $dir_name:literal => $dir_contents:tt)?
        $(file $file:expr)?
    ),*) => {
      [
            $(
                $(
                    InitStateItem::create_directory($dir_name, &generate_init_state! $dir_contents),
                )?
                $(
                    InitStateItem::create_file($file),
                )?
            )*
        ]
    };
}

impl<T: ProcFSFile + Clone + 'static> IntoBoxFile for T {
    fn into_box(&self) -> Box<dyn ProcFSFile> {
        Box::new(Clone::clone(self))
    }
}

static CPU_INFO: CpuInfoWrapper = cpuinfo::CpuInfoWrapper::new();

pub static INIT_STATE: &[InitStateItem<'static>] = &generate_init_state! {
    file &CPU_INFO,
};
