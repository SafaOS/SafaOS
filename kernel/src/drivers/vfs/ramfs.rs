use core::str::FromStr;

use crate::utils::types::Name;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use hashbrown::HashMap;
use spin::{Mutex, RwLock};

use crate::devices::Device;
use crate::memory::page_allocator::{PageAlloc, GLOBAL_PAGE_ALLOCATOR};
use crate::utils::path::PathParts;

use super::{DirIterInodeItem, FileName, InodeOf};
use super::{FSError, FSResult, FileSystem, Inode, InodeOps, InodeType};

/// The data of a RamInode
// you cannot just lock the whole enum because Devices' manage their own locks
pub enum RamInodeData {
    Data(Mutex<Vec<u8, PageAlloc>>),
    Children(Mutex<HashMap<FileName, usize>>),
    HardLink(Inode),
    Device(&'static dyn Device),
}

pub struct RamInode {
    data: RamInodeData,
    inodeid: usize,
}
impl RamInode {
    fn new(data: RamInodeData, inodeid: usize) -> Self {
        Self { data, inodeid }
    }

    fn new_file(inodeid: usize) -> InodeOf<Self> {
        Arc::new(RamInode::new(
            RamInodeData::Data(Mutex::new(Vec::new_in(&*GLOBAL_PAGE_ALLOCATOR))),
            inodeid,
        ))
    }

    fn new_dir(inodeid: usize) -> InodeOf<Self> {
        Arc::new(RamInode::new(
            RamInodeData::Children(Mutex::new(HashMap::new())),
            inodeid,
        ))
    }

    fn new_device(device: &'static dyn Device, inodeid: usize) -> InodeOf<Self> {
        Arc::new(RamInode::new(RamInodeData::Device(device), inodeid))
    }

    fn new_hardlink(inode: Inode, inodeid: usize) -> InodeOf<Self> {
        Arc::new(RamInode::new(RamInodeData::HardLink(inode), inodeid))
    }
}

impl InodeOps for RamInode {
    fn size(&self) -> FSResult<usize> {
        match self.data {
            RamInodeData::Data(ref data) => Ok(data.lock().len()),
            RamInodeData::Device(ref device) => device.size(),
            _ => Err(FSError::NotAFile),
        }
    }
    fn get(&self, name: &str) -> FSResult<usize> {
        match self.data {
            RamInodeData::Children(ref tree) => tree
                .lock()
                .get(name)
                .copied()
                .ok_or(FSError::NoSuchAFileOrDirectory),
            RamInodeData::HardLink(ref inode) => inode.get(name),
            RamInodeData::Device(ref device) => device.get(name),
            _ => Err(FSError::NotADirectory),
        }
    }

    fn contains(&self, name: &str) -> bool {
        match self.data {
            RamInodeData::Children(ref tree) => tree.lock().contains_key(name),
            RamInodeData::HardLink(ref inode) => inode.contains(name),
            RamInodeData::Device(ref device) => device.contains(name),
            _ => false,
        }
    }

    fn truncate(&self, size: usize) -> FSResult<()> {
        match self.data {
            RamInodeData::Data(ref data) => {
                data.lock().truncate(size);
                Ok(())
            }
            RamInodeData::HardLink(ref inode) => inode.truncate(size),
            RamInodeData::Device(ref device) => device.truncate(size),
            _ => Err(FSError::NotAFile),
        }
    }

    fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        match self.data {
            RamInodeData::Data(ref data) => {
                let data = data.lock();
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
                    let len = data.len();
                    if rev_offset > len + 1 {
                        return Err(FSError::InvaildOffset);
                    }

                    drop(data);
                    // TODO: this is slower then inlining the code ourselves
                    self.read(((len + 1) - rev_offset) as isize, buffer)
                }
            }
            RamInodeData::HardLink(ref inode) => inode.read(offset, buffer),
            RamInodeData::Device(ref device) => device.read(offset, buffer),
            _ => Err(FSError::NotAFile),
        }
    }

    fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        match self.data {
            RamInodeData::Data(ref data) => {
                let mut data = data.lock();

                if offset >= 0 {
                    let offset = offset as usize;
                    if data.len() < buffer.len() + offset {
                        data.resize(buffer.len() + offset, 0);
                    }

                    data[offset..(offset + buffer.len())].copy_from_slice(buffer);
                    Ok(buffer.len())
                } else {
                    let rev_offset = (-offset) as usize;
                    let len = data.len();

                    if rev_offset > len + 1 {
                        return Err(FSError::InvaildOffset);
                    }

                    drop(data);
                    self.write(((len + 1) - rev_offset) as isize, buffer)
                }
            }
            RamInodeData::HardLink(ref inode) => inode.write(offset, buffer),
            RamInodeData::Device(ref device) => device.write(offset, buffer),
            _ => Err(FSError::NotAFile),
        }
    }

    fn insert(&self, name: Name, node: usize) -> FSResult<()> {
        match self.data {
            RamInodeData::Children(ref tree) => {
                let mut tree = tree.lock();
                let name = name.into();

                if tree.contains_key(&name) {
                    return Err(FSError::AlreadyExists);
                }

                tree.insert(name, node);
                Ok(())
            }
            RamInodeData::HardLink(ref inode) => inode.insert(name, node),
            RamInodeData::Device(ref device) => device.insert(name, node),
            _ => Err(FSError::NotADirectory),
        }
    }

    fn kind(&self) -> InodeType {
        match self.data {
            RamInodeData::Children(_) => InodeType::Directory,
            RamInodeData::Data(_) => InodeType::File,
            RamInodeData::Device(_) => InodeType::Device,
            RamInodeData::HardLink(ref inode) => inode.kind(),
        }
    }

    fn inodeid(&self) -> usize {
        self.inodeid
    }
    fn open_diriter(&self) -> FSResult<Box<[DirIterInodeItem]>> {
        match self.data {
            RamInodeData::Children(ref data) => {
                let data = data.lock();
                Ok(data
                    .iter()
                    .map(|(name, inodeid)| (name.clone(), *inodeid))
                    .collect())
            }
            RamInodeData::HardLink(ref inode) => inode.open_diriter(),
            RamInodeData::Device(ref device) => device.open_diriter(),
            _ => Err(FSError::NotADirectory),
        }
    }

    fn sync(&self) -> FSResult<()> {
        match self.data {
            RamInodeData::HardLink(ref inode) => inode.sync(),
            RamInodeData::Device(ref device) => device.sync(),
            _ => Ok(()),
        }
    }

    fn ctl<'a>(&'a self, cmd: u16, args: super::CtlArgs<'a>) -> FSResult<()> {
        match self.data {
            RamInodeData::HardLink(ref inode) => inode.ctl(cmd, args),
            RamInodeData::Device(ref device) => device.ctl(cmd, args),
            _ => Err(FSError::OperationNotSupported),
        }
    }
}

pub struct RamFS {
    inodes: Vec<Arc<RamInode>>,
}

impl RamFS {
    pub fn new() -> Self {
        Self {
            inodes: vec![RamInode::new_dir(0)],
        }
    }

    fn make_hardlink(&mut self, pointer_inodeid: usize) -> usize {
        let inodeid = self.inodes.len();

        let pointer_inode = self.inodes.get_mut(pointer_inodeid).unwrap();
        let pointer_inode = pointer_inode.clone();

        self.inodes
            .push(RamInode::new_hardlink(pointer_inode, inodeid));
        inodeid
    }

    fn make_file(&mut self) -> usize {
        let inodeid = self.inodes.len();
        let node = RamInode::new_file(inodeid);
        self.inodes.push(node.clone());
        inodeid
    }

    fn make_device(&mut self, device: &'static dyn Device) -> usize {
        let inodeid = self.inodes.len();
        let node = RamInode::new_device(device, inodeid);
        self.inodes.push(node.clone());
        inodeid
    }

    fn make_directory(&mut self) -> Inode {
        let inodeid = self.inodes.len();
        let node = RamInode::new_dir(inodeid);
        self.inodes.push(node.clone());
        node
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

    fn create(&self, path: PathParts) -> FSResult<()> {
        let (parent, name) = self.reslove_path_uncreated(path)?;
        let name = Name::from_str(name).map_err(|()| FSError::InvaildName)?;

        let mut write = self.write();
        let new_node = write.make_file();

        parent.insert(name, new_node)?;
        Ok(())
    }

    fn createdir(&self, path: PathParts) -> FSResult<()> {
        let (parent, name) = self.reslove_path_uncreated(path)?;
        let name = Name::from_str(name).map_err(|()| FSError::InvaildName)?;

        let mut write = self.write();
        let new_node = write.make_directory();

        // inserting the new dir in the parent dir
        parent.insert(name, new_node.inodeid())?;
        // making the previous dir inode a hardlink
        let parent_hardlink_inodeid = write.make_hardlink(parent.inodeid());
        // inserting the previous dir
        let hardlink_name = unsafe { Name::from_str("..").unwrap_unchecked() };
        new_node.insert(hardlink_name, parent_hardlink_inodeid)?;
        Ok(())
    }

    fn mount_device(&self, path: PathParts, device: &'static dyn Device) -> FSResult<()> {
        let (parent, name) = self.reslove_path_uncreated(path)?;
        let name = Name::from_str(name).map_err(|()| FSError::InvaildName)?;

        let mut write = self.write();
        let new_node = write.make_device(device);

        parent.insert(name, new_node)?;
        Ok(())
    }
}
