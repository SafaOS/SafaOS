use core::str;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use hashbrown::HashMap;
use init::InitStateItem;
use spin::Mutex;
use tasks::TaskInfoFile;

use crate::{
    threading::{expose::getpids, Pid},
    utils::alloc::PageString,
};

use super::{FSError, FSResult, Inode, InodeOps};

mod cpuinfo;
mod init;
mod kernelinfo;
mod meminfo;
mod tasks;

pub struct ProcFSFile {
    name: &'static str,
    id: usize,
    data: Option<PageString>,
    /// if true the data won't be de-allocated when the file is closed
    is_static: bool,
    fetch: fn(&mut Self) -> Option<PageString>,
}

impl ProcFSFile {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn new(name: &'static str, id: usize, fetch: fn(&mut Self) -> Option<PageString>) -> Self {
        Self {
            name,
            id,
            data: None,
            is_static: false,
            fetch,
        }
    }

    pub fn new_static(
        name: &'static str,
        id: usize,
        fetch: fn(&mut Self) -> Option<PageString>,
    ) -> Self {
        Self {
            name,
            id,
            data: None,
            is_static: true,
            fetch,
        }
    }

    fn get_data(&mut self) -> &str {
        if self.data.is_none() {
            self.refresh();
        }

        self.data.as_ref().unwrap().as_str()
    }

    fn close(&mut self) {
        if !self.is_static {
            self.data = None;
        }
    }

    fn refresh(&mut self) {
        let fetch = self.fetch;
        self.data = fetch(self);
    }
}

pub struct ProcInode {
    inodeid: usize,
    data: ProcInodeData,
}

enum ProcInodeData {
    Dir(String, HashMap<String, usize>),
    File(ProcFSFile),
}

impl ProcInode {
    fn new_dir(inodeid: usize, name: String) -> Self {
        Self {
            inodeid,
            data: ProcInodeData::Dir(name, HashMap::new()),
        }
    }

    fn new_file(inodeid: usize, data: ProcFSFile) -> Self {
        Self {
            inodeid,
            data: ProcInodeData::File(data),
        }
    }
}

impl super::InodeOps for Mutex<ProcInode> {
    fn inodeid(&self) -> usize {
        self.lock().inodeid
    }

    fn kind(&self) -> super::InodeType {
        match &self.lock().data {
            ProcInodeData::Dir(_, _) => super::InodeType::Directory,
            ProcInodeData::File(_) => super::InodeType::File,
        }
    }

    fn contains(&self, name: &str) -> bool {
        match &self.lock().data {
            ProcInodeData::Dir(_, dir) => dir.contains_key(name),
            ProcInodeData::File(_) => false,
        }
    }

    fn size(&self) -> FSResult<usize> {
        match &mut self.lock().data {
            ProcInodeData::Dir(_, dir) => Ok(dir.len()),
            ProcInodeData::File(file) => Ok(file.get_data().len()),
        }
    }

    fn get(&self, name: &str) -> FSResult<usize> {
        match &self.lock().data {
            ProcInodeData::Dir(_, dir) => dir
                .get(name)
                .copied()
                .ok_or(FSError::NoSuchAFileOrDirectory),
            ProcInodeData::File(_) => Err(FSError::NotADirectory),
        }
    }

    fn name(&self) -> String {
        match &self.lock().data {
            ProcInodeData::Dir(name, _) => name.clone(),
            ProcInodeData::File(file) => file.name().to_string(),
        }
    }

    fn read(&self, buffer: &mut [u8], offset: usize, count: usize) -> FSResult<usize> {
        match &mut self.lock().data {
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

    fn open_diriter(&self) -> FSResult<Box<[usize]>> {
        match &self.lock().data {
            ProcInodeData::Dir(_, dir) => {
                let mut inodeids = Vec::with_capacity(dir.len());
                for inodeid in dir.values() {
                    inodeids.push(*inodeid);
                }

                Ok(inodeids.into_boxed_slice())
            }
            _ => FSResult::Err(FSError::NotADirectory),
        }
    }

    fn insert(&self, name: &str, node: usize) -> FSResult<()> {
        match &mut self.lock().data {
            ProcInodeData::Dir(_, dir) => {
                if dir.contains_key(name) {
                    return FSResult::Err(FSError::AlreadyExists);
                }

                dir.insert(name.to_string(), node);
                Ok(())
            }
            ProcInodeData::File(_) => FSResult::Err(FSError::NotADirectory),
        }
    }

    fn close(&self) {
        match &mut self.lock().data {
            ProcInodeData::Dir(_, _) => {}
            ProcInodeData::File(file) => file.close(),
        }
    }

    fn opened(&self) {
        match &mut self.lock().data {
            ProcInodeData::Dir(_, _) => {}
            ProcInodeData::File(file) => file.refresh(),
        }
    }
}
pub struct ProcFS {
    /// inodeid -> inode
    inodes: HashMap<usize, Arc<Mutex<ProcInode>>>,
    /// pid -> task inodeid
    tasks: HashMap<usize, usize>,

    next_inodeid: usize,
}

impl ProcFS {
    pub fn new() -> Self {
        Self {
            inodes: HashMap::from([(
                0,
                Arc::new(Mutex::new(ProcInode::new_dir(0, String::new()))),
            )]),
            tasks: HashMap::new(),
            next_inodeid: 1,
        }
    }

    fn append_init_state(&mut self, state: InitStateItem) -> (&'static str, usize) {
        match state {
            InitStateItem::File(file) => self.append_file(file),
        }
    }

    /// Creates a new procfs with the init state defined in [`self::init`]
    pub fn create() -> Self {
        let mut fs = Self::new();
        let root_inode = fs.inodes.get(&0).unwrap().clone();

        for item in init::get_init_state() {
            let (name, inodeid) = fs.append_init_state(item);
            root_inode.insert(name, inodeid).unwrap();
        }
        fs
    }

    fn append_file(&mut self, file: ProcFSFile) -> (&'static str, usize) {
        let name = file.name();

        let inodeid = self.next_inodeid;
        self.next_inodeid += 1;

        self.inodes.insert(
            inodeid,
            Arc::new(Mutex::new(ProcInode::new_file(inodeid, file))),
        );

        (name, inodeid)
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
            Arc::new(Mutex::new(ProcInode {
                inodeid,
                data: ProcInodeData::Dir(name.to_string(), dir),
            })),
        );

        let root_inode = self.inodes.get(&0).unwrap();
        root_inode.insert(name, inodeid).unwrap();
        inodeid
    }

    fn append_process(&mut self, pid: Pid) -> usize {
        let info_file = TaskInfoFile::new(pid);
        let (file_name, file_inode) = self.append_file(info_file);

        let inodeid = self.append_dir(&pid.to_string(), &[(file_name, file_inode)]);
        self.tasks.insert(pid, inodeid);
        inodeid
    }

    fn remove_inode(&mut self, inodeid: usize) {
        let inode = self.inodes.remove(&inodeid).unwrap();
        match &inode.lock().data {
            ProcInodeData::Dir(_, dir) => {
                for (_, inodeid) in dir {
                    self.remove_inode(*inodeid);
                }
            }
            ProcInodeData::File(_) => {}
        }

        drop(inode);

        for (pid, p_inodeid) in self.tasks.iter() {
            if inodeid == *p_inodeid {
                let pid = *pid;
                self.tasks.remove(&pid);
                break;
            }
        }
    }

    pub fn update_processes(&mut self) {
        let getpids = getpids();
        // O(N)
        for pid in &getpids {
            if !self.tasks.contains_key(pid) {
                self.append_process(*pid);
            }
        }

        // O(NlogN)
        let useless_inodes: Vec<_> = self
            .tasks
            .extract_if(|pid, _| getpids.binary_search(pid).is_err())
            .collect();

        for (pid, inodeid) in useless_inodes {
            self.remove_inode(inodeid);
            match &mut self.inodes.get(&0).unwrap().lock().data {
                ProcInodeData::Dir(_, dir) => {
                    dir.remove(&pid.to_string());
                }
                ProcInodeData::File(_) => unreachable!(),
            }
        }
    }
}

impl super::FileSystem for Mutex<ProcFS> {
    fn on_open(&self, _: super::Path) -> FSResult<()> {
        self.lock().update_processes();
        Ok(())
    }

    fn name(&self) -> &'static str {
        "proc"
    }

    fn get_inode(&self, inode_id: usize) -> Option<Inode> {
        self.lock()
            .inodes
            .get(&inode_id)
            .cloned()
            .map(|x| x as Inode)
    }
}
