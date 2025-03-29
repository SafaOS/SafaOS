use core::{
    fmt::Write,
    str::{self, FromStr},
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use hashbrown::HashMap;
use init::InitStateItem;
use safa_utils::types::Name;
use spin::Mutex;
use tasks::TaskInfoFile;

use crate::{
    threading::{self, Pid},
    utils::alloc::PageString,
};

use super::{DirIterInodeItem, FSError, FSResult, FileName, Inode, InodeOps};

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
    Dir(HashMap<FileName, usize>),
    File(ProcFSFile),
}

impl ProcInode {
    fn new_dir(inodeid: usize, data: HashMap<FileName, usize>) -> Self {
        Self {
            inodeid,
            data: ProcInodeData::Dir(data),
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
            ProcInodeData::Dir(_) => super::InodeType::Directory,
            ProcInodeData::File(_) => super::InodeType::File,
        }
    }

    fn contains(&self, name: &str) -> bool {
        match &self.lock().data {
            ProcInodeData::Dir(dir) => dir.contains_key(name),
            ProcInodeData::File(_) => false,
        }
    }

    fn size(&self) -> FSResult<usize> {
        match &mut self.lock().data {
            ProcInodeData::Dir(dir) => Ok(dir.len()),
            ProcInodeData::File(file) => Ok(file.get_data().len()),
        }
    }

    fn get(&self, name: &str) -> FSResult<usize> {
        match &self.lock().data {
            ProcInodeData::Dir(dir) => dir
                .get(name)
                .copied()
                .ok_or(FSError::NoSuchAFileOrDirectory),
            ProcInodeData::File(_) => Err(FSError::NotADirectory),
        }
    }

    fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        match &mut self.lock().data {
            ProcInodeData::File(file) => {
                let file_data = file.get_data();
                if offset >= file_data.len() as isize {
                    return Err(FSError::InvaildOffset);
                }

                if offset >= 0 {
                    let offset = offset as usize;
                    let count = buffer.len().min(file_data.len() - offset);

                    buffer[..count].copy_from_slice(file_data[offset..offset + count].as_bytes());
                    Ok(count)
                } else {
                    let rev_offset = (-offset) as usize;
                    if rev_offset > file_data.len() {
                        return Err(FSError::InvaildOffset);
                    }
                    // TODO: this is slower then inlining the code ourselves
                    self.read((file_data.len() - rev_offset) as isize + 1, buffer)
                }
            }
            _ => FSResult::Err(FSError::NotAFile),
        }
    }

    fn open_diriter(&self) -> FSResult<Box<[DirIterInodeItem]>> {
        match &self.lock().data {
            ProcInodeData::Dir(dir) => {
                let mut inodeids = Vec::with_capacity(dir.len());
                for (name, inodeid) in dir {
                    inodeids.push((Name::from_str(name).unwrap().into(), *inodeid));
                }

                Ok(inodeids.into_boxed_slice())
            }
            _ => FSResult::Err(FSError::NotADirectory),
        }
    }

    fn insert(&self, name: Name, node: usize) -> FSResult<()> {
        match &mut self.lock().data {
            ProcInodeData::Dir(dir) => {
                let name = name.into();
                if dir.contains_key(&name) {
                    return FSResult::Err(FSError::AlreadyExists);
                }

                dir.insert(name, node);
                Ok(())
            }
            ProcInodeData::File(_) => FSResult::Err(FSError::NotADirectory),
        }
    }

    fn close(&self) {
        match &mut self.lock().data {
            ProcInodeData::Dir(_) => {}
            ProcInodeData::File(file) => file.close(),
        }
    }

    fn opened(&self) {
        match &mut self.lock().data {
            ProcInodeData::Dir(_) => {}
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
                Arc::new(Mutex::new(ProcInode::new_dir(0, HashMap::new()))),
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
            let name = Name::from_str(name).unwrap();

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

    fn append_dir(&mut self, name: Name, items: &[(&str, usize)]) -> usize {
        let inodeid = self.next_inodeid;
        self.next_inodeid += 1;

        let data = HashMap::from_iter(
            items
                .iter()
                .map(|(name, inodeid)| (Name::from_str(name).unwrap().into(), *inodeid)),
        );

        self.inodes.insert(
            inodeid,
            Arc::new(Mutex::new(ProcInode::new_dir(inodeid, data))),
        );

        let root_inode = self.inodes.get(&0).unwrap();
        root_inode.insert(name, inodeid).unwrap();
        inodeid
    }

    fn append_process(&mut self, pid: Pid) -> usize {
        let info_file = TaskInfoFile::new(pid);
        let (info_file_name, info_file_inode) = self.append_file(info_file);

        let mut pid_str = Name::new();
        pid_str.write_fmt(format_args!("{}", pid)).unwrap();

        let inodeid = self.append_dir(pid_str, &[(info_file_name, info_file_inode)]);
        self.tasks.insert(pid, inodeid);
        inodeid
    }

    fn remove_inode(&mut self, inodeid: usize) {
        let inode = self.inodes.remove(&inodeid).unwrap();
        match &inode.lock().data {
            ProcInodeData::Dir(dir) => {
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
        let schd = threading::schd();
        let getpids = schd.pids();
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
                ProcInodeData::Dir(dir) => {
                    let mut pid_str = Name::new();
                    pid_str.write_fmt(format_args!("{}", pid)).unwrap();

                    let pid_str: FileName = pid_str.into();
                    dir.remove(&pid_str);
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
