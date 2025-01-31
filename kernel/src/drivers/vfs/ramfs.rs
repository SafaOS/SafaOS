use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec;
use alloc::{collections::btree_map::BTreeMap, string::String, vec::Vec};
use spin::{Mutex, RwLock};

use crate::memory::page_allocator::{PageAlloc, GLOBAL_PAGE_ALLOCATOR};

use super::InodeOf;
use super::{FSError, FSResult, FileSystem, Inode, InodeOps, InodeType, Path};

pub enum RamInodeData {
    Data(Vec<u8, PageAlloc>),
    Children(BTreeMap<String, usize>),
    HardLink(Inode),
}

pub struct RamInode {
    name: String,
    data: RamInodeData,
    inodeid: usize,
}
impl RamInode {
    fn new(name: String, data: RamInodeData, inodeid: usize) -> Mutex<Self> {
        Mutex::new(Self {
            name,
            data,
            inodeid,
        })
    }

    fn new_file(name: String, data: &[u8], inodeid: usize) -> InodeOf<Mutex<Self>> {
        Arc::new(RamInode::new(
            name,
            RamInodeData::Data(data.to_vec_in(&*GLOBAL_PAGE_ALLOCATOR)),
            inodeid,
        ))
    }

    fn new_dir(name: String, inodeid: usize) -> InodeOf<Mutex<Self>> {
        Arc::new(RamInode::new(
            name,
            RamInodeData::Children(BTreeMap::new()),
            inodeid,
        ))
    }

    fn new_hardlink(name: String, inode: Inode, inodeid: usize) -> InodeOf<Mutex<Self>> {
        Arc::new(RamInode::new(name, RamInodeData::HardLink(inode), inodeid))
    }
}

impl InodeOps for Mutex<RamInode> {
    fn size(&self) -> FSResult<usize> {
        match self.lock().data {
            RamInodeData::Data(ref data) => Ok(data.len()),
            _ => Err(FSError::NotAFile),
        }
    }
    fn get(&self, name: Path) -> FSResult<usize> {
        match self.lock().data {
            RamInodeData::Children(ref tree) => tree
                .get(name)
                .copied()
                .ok_or(FSError::NoSuchAFileOrDirectory),
            RamInodeData::HardLink(ref inode) => inode.get(name),
            _ => Err(FSError::NotADirectory),
        }
    }

    fn contains(&self, name: Path) -> bool {
        match self.lock().data {
            RamInodeData::Children(ref tree) => tree.contains_key(name),
            RamInodeData::HardLink(ref inode) => inode.contains(name),
            _ => false,
        }
    }

    fn truncate(&self, size: usize) -> FSResult<()> {
        match self.lock().data {
            RamInodeData::Data(ref mut data) => {
                data.truncate(size);
                Ok(())
            }
            RamInodeData::HardLink(ref inode) => inode.truncate(size),
            _ => Err(FSError::NotAFile),
        }
    }

    fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        match self.lock().data {
            RamInodeData::Data(ref data) => {
                if offset >= data.len() as isize {
                    return Err(FSError::InvaildOffset);
                }

                if offset >= 0 {
                    let offset = offset as usize;

                    let count = buffer.len().min(data.len() - offset);
                    buffer[..count].copy_from_slice(&data[offset..offset + count]);
                    Ok(count)
                } else {
                    let rev_offset = (-offset) as usize;
                    if rev_offset > data.len() {
                        return Err(FSError::InvaildOffset);
                    }
                    // TODO: this is slower then inlining the code ourselves
                    self.read((data.len() - rev_offset) as isize + 1, buffer)
                }
            }
            RamInodeData::HardLink(ref inode) => inode.read(offset, buffer),
            _ => Err(FSError::NotAFile),
        }
    }

    fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        match self.lock().data {
            RamInodeData::Data(ref mut data) => {
                if offset >= 0 {
                    let offset = offset as usize;
                    if data.len() < buffer.len() + offset {
                        data.resize(buffer.len() + offset, 0);
                    }

                    data[offset..(offset + buffer.len())].copy_from_slice(buffer);
                    Ok(buffer.len())
                } else {
                    let rev_offset = (-offset) as usize;
                    if rev_offset > data.len() {
                        return Err(FSError::InvaildOffset);
                    }
                    // TODO: this is slower then inlining the code ourselves
                    self.write((data.len() - rev_offset) as isize + 1, buffer)
                }
            }
            RamInodeData::HardLink(ref inode) => inode.write(offset, buffer),
            _ => Err(FSError::NotAFile),
        }
    }

    fn insert(&self, name: &str, node: usize) -> FSResult<()> {
        match self.lock().data {
            RamInodeData::Children(ref mut tree) => {
                if tree.contains_key(name) {
                    return Err(FSError::AlreadyExists);
                }

                tree.insert(name.to_string(), node);
                Ok(())
            }
            RamInodeData::HardLink(ref inode) => inode.insert(name, node),
            _ => Err(FSError::NotADirectory),
        }
    }

    fn kind(&self) -> InodeType {
        match self.lock().data {
            RamInodeData::Children(_) => InodeType::Directory,
            RamInodeData::Data(_) => InodeType::File,
            RamInodeData::HardLink(ref inode) => inode.kind(),
        }
    }

    fn name(&self) -> String {
        self.lock().name.clone()
    }

    fn inodeid(&self) -> usize {
        self.lock().inodeid
    }
    fn open_diriter(&self) -> FSResult<Box<[usize]>> {
        match self.lock().data {
            RamInodeData::Children(ref data) => {
                Ok(data.iter().map(|(_, inodeid)| *inodeid).collect())
            }

            RamInodeData::HardLink(ref inode) => inode.open_diriter(),
            _ => Err(FSError::NotADirectory),
        }
    }
}

pub struct RamFS {
    inodes: Vec<Arc<Mutex<RamInode>>>,
}

impl RamFS {
    pub fn new() -> Self {
        Self {
            inodes: vec![RamInode::new_dir("/".to_string(), 0)],
        }
    }

    fn make_hardlink(&mut self, inodeid: usize, name: String) -> usize {
        let inode = self.inodes.get_mut(inodeid).unwrap();
        let inode = inode.clone();
        let inodeid = self.inodes.len();

        self.inodes
            .push(RamInode::new_hardlink(name, inode, inodeid));
        inodeid
    }
}

impl FileSystem for RwLock<RamFS> {
    fn name(&self) -> &'static str {
        "ramfs"
    }

    #[inline]
    fn get_inode(&self, inode_id: usize) -> Option<Inode> {
        self.read()
            .inodes
            .get(inode_id)
            .cloned()
            .map(|x| x as Inode)
    }

    fn create(&self, path: Path) -> FSResult<()> {
        let inodeid = self.read().inodes.len();

        let (resloved, name) = self.reslove_path_uncreated(path)?;
        resloved.insert(name, inodeid)?;

        let node = RamInode::new_file(name.to_string(), &[], inodeid);
        self.write().inodes.push(node);

        Ok(())
    }

    fn createdir(&self, path: Path) -> FSResult<()> {
        let inodeid = self.read().inodes.len();

        let (resloved, name) = self.reslove_path_uncreated(path)?;
        resloved.insert(name, inodeid)?;

        let node = RamInode::new_dir(name.to_string(), inodeid);
        self.write().inodes.push(node.clone());

        let inodeid = self
            .write()
            .make_hardlink(resloved.inodeid(), "..".to_string());
        node.insert("..", inodeid)?;

        Ok(())
    }
}
