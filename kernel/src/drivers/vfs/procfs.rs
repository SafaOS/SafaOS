use core::str;

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use hashbrown::HashMap;

use crate::threading::{
    expose::{getinfo, getpid, getpids},
    processes::ProcessInfo,
};

use super::{DirIter, FSError, FSResult, FileDescriptor, Inode, FS};

pub trait ProcFSFile: Send + Sync {
    /// returns the name of the file which is statically known
    fn name(&self) -> &'static str;
    /// returns the data in the file which is a utf8 string representing Self as json
    fn get_data(&self) -> String;
}

struct ProcessInfoFile(usize);

impl ProcFSFile for ProcessInfoFile {
    fn name(&self) -> &'static str {
        "info"
    }
    fn get_data(&self) -> String {
        todo!()
    }
}

struct ProcInode {
    inodeid: usize,
    data: ProcInodeData,
}

enum ProcInodeData {
    Dir(String, HashMap<String, usize>),
    File(Box<dyn ProcFSFile>),
}

impl ProcInode {
    fn new_dir(inodeid: usize, name: String) -> Self {
        Self {
            inodeid,
            data: ProcInodeData::Dir(name, HashMap::new()),
        }
    }

    fn new_file(inodeid: usize, data: Box<dyn ProcFSFile>) -> Self {
        Self {
            inodeid,
            data: ProcInodeData::File(data),
        }
    }
}

impl super::InodeOps for ProcInode {
    fn inodeid(&self) -> usize {
        self.inodeid
    }

    fn kind(&self) -> super::InodeType {
        match &self.data {
            ProcInodeData::Dir(_, _) => super::InodeType::Directory,
            ProcInodeData::File(_) => super::InodeType::File,
        }
    }

    fn size(&self) -> FSResult<usize> {
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn name(&self) -> String {
        match &self.data {
            ProcInodeData::Dir(name, _) => name.clone(),
            ProcInodeData::File(file) => file.name().to_string(),
        }
    }

    fn read(&self, buffer: &mut [u8], offset: usize, count: usize) -> FSResult<usize> {
        match &self.data {
            ProcInodeData::File(file) => {
                let file_data = file.get_data();
                let count = if offset + count > file_data.len() {
                    file_data.len() - offset
                } else {
                    count
                };

                buffer[..count].copy_from_slice(file_data[offset..offset + count].as_bytes());
                Ok(count)
            }
            _ => FSResult::Err(FSError::NotAFile),
        }
    }

    fn open_diriter(&self, fs: *mut dyn FS) -> FSResult<DirIter> {
        match &self.data {
            ProcInodeData::Dir(_, dir) => {
                let mut inodeids = Vec::with_capacity(dir.len());
                for (_, inodeid) in dir {
                    inodeids.push(*inodeid);
                }

                Ok(DirIter::new(fs, inodeids.into_boxed_slice()))
            }
            _ => FSResult::Err(FSError::NotADirectory),
        }
    }
}
pub struct ProcFS {
    /// inodeid -> inode
    inodes: HashMap<usize, Arc<ProcInode>>,
    /// pid -> process inodeid
    processes: HashMap<usize, usize>,

    next_inodeid: usize,
}

impl ProcFS {
    pub fn new() -> Self {
        Self {
            inodes: HashMap::from([(0, Arc::new(ProcInode::new_dir(0, String::new())))]),
            processes: HashMap::new(),
            next_inodeid: 1,
        }
    }

    fn append_file(&mut self, file: Box<dyn ProcFSFile>) -> (usize, &'static str) {
        let name = file.name();
        let inodeid = self.next_inodeid;
        self.next_inodeid += 1;
        self.inodes
            .insert(inodeid, Arc::new(ProcInode::new_file(inodeid, file)));

        (inodeid, name)
    }

    fn append_dir(&mut self, name: &str, items: &[(&str, usize)]) -> usize {
        let inodeid = self.next_inodeid;
        self.next_inodeid += 1;
        let dir = HashMap::from_iter(
            items
                .iter()
                .map(|(name, inodeid)| (name.to_string(), *inodeid)),
        );

        self.inodes.insert(
            inodeid,
            Arc::new(ProcInode {
                inodeid,
                data: ProcInodeData::Dir(name.to_string(), dir),
            }),
        );
        self.root_inode().unwrap().insert(name, inodeid).unwrap();
        inodeid
    }

    fn append_process(&mut self, pid: usize) -> usize {
        let info_file = Box::new(ProcessInfoFile(pid));
        let (file_inode, file_name) = self.append_file(info_file);

        let inodeid = self.append_dir(&pid.to_string(), &[(file_name, file_inode)]);
        self.processes.insert(pid, inodeid);
        inodeid
    }

    fn remove_inode(&mut self, inodeid: usize) {
        let inode = self.inodes.remove(&inodeid).unwrap();
        match &inode.data {
            ProcInodeData::Dir(_, dir) => {
                for (_, inodeid) in dir {
                    self.remove_inode(*inodeid);
                }
            }
            ProcInodeData::File(_) => {}
        }

        drop(inode);

        for (pid, p_inodeid) in self.processes.iter() {
            if inodeid == *p_inodeid {
                let pid = *pid;
                self.processes.remove(&pid);
                break;
            }
        }
    }

    pub fn update_processes(&mut self) {
        let getpids = getpids();
        // O(N)
        for pid in &getpids {
            if !self.processes.contains_key(pid) {
                self.append_process(*pid);
            }
        }

        // O(NlogN)
        let useless_inodes: Vec<_> = self
            .processes
            .extract_if(|pid, _| getpids.binary_search(pid).is_ok())
            .map(|(_, inodeid)| inodeid)
            .collect();

        for inodeid in useless_inodes {
            self.remove_inode(inodeid);
        }
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
        self.inodes
            .get(&0)
            .cloned()
            .map(|inode| inode as Arc<dyn super::InodeOps>)
            .ok_or(FSError::NoSuchAFileOrDirectory)
    }

    fn diriter_open(&mut self, fd: &mut FileDescriptor) -> FSResult<DirIter> {
        if fd.node.inodeid() == 0 {
            self.update_processes();
        }

        fd.node.open_diriter(self as *const Self as *mut Self)
    }
}
