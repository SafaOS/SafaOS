use core::str;

use alloc::{format, string::String, sync::Arc, vec::Vec};

use crate::threading::{
    expose::{getinfo, getpids},
    processes::ProcessInfo,
};

use super::{DirIter, FSError, FSResult, FileDescriptor, Inode};

pub struct ProcFS;
#[derive(Clone)]
pub struct ProcInode(ProcessInfo);
pub struct RootProcessInode;

impl super::InodeOps for ProcInode {
    fn inodeid(&self) -> usize {
        self.0.pid as usize + 1
    }

    fn kind(&self) -> super::InodeType {
        super::InodeType::Device
    }

    fn name(&self) -> String {
        format!("{}", self.0.pid)
    }

    fn contains(&self, _: &str) -> bool {
        false
    }

    fn get(&self, _: &str) -> FSResult<usize> {
        Err(FSError::NotADirectory)
    }
}

impl ProcInode {
    pub fn new(process: ProcessInfo) -> Inode {
        Arc::new(Self(process))
    }
}

impl super::InodeOps for RootProcessInode {
    fn inodeid(&self) -> usize {
        0
    }

    fn kind(&self) -> super::InodeType {
        super::InodeType::Directory
    }

    fn name(&self) -> String {
        String::from("")
    }

    fn open_diriter(&self, fs: *mut dyn super::FS) -> FSResult<DirIter> {
        let inodeids = getpids()
            .iter()
            .map(|pid| (pid + 1) as usize)
            .collect::<Vec<_>>();

        Ok(DirIter::new(fs, inodeids.into_boxed_slice()))
    }
}
impl ProcFS {
    pub fn new() -> Self {
        Self
    }
}

impl super::FS for ProcFS {
    fn name(&self) -> &'static str {
        "proc"
    }

    fn open(&self, path: super::Path) -> FSResult<FileDescriptor> {
        let file = self.reslove_path(path)?;
        let node = file.clone();

        Ok(FileDescriptor::new(self as *const Self as *mut Self, node))
    }
    fn root_inode(&self) -> FSResult<Inode> {
        Ok(Arc::new(RootProcessInode))
    }

    fn get_inode(&self, inode_id: usize) -> FSResult<Option<Inode>> {
        if inode_id == 0 {
            return Ok(Some(self.root_inode()?));
        }

        let pid = inode_id - 1;
        Ok(getinfo(pid).map(|process| ProcInode::new(process)))
    }
}
