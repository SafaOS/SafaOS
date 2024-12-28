use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use spin::Mutex;

use crate::devices::{Device, DEVICE_MANAGER};

use super::{DirIter, FSResult, FileDescriptor, Inode, InodeOps, InodeType, Path, FS};

pub struct DeviceManagerInode;
impl InodeOps for Mutex<DeviceManagerInode> {
    fn inodeid(&self) -> usize {
        0
    }

    fn kind(&self) -> InodeType {
        InodeType::Directory
    }

    fn name(&self) -> String {
        String::new()
    }

    fn contains(&self, name: &str) -> bool {
        for device in DEVICE_MANAGER.lock().devices().iter() {
            if Device::name(*device) == name {
                return true;
            }
        }
        false
    }

    fn get(&self, name: &str) -> crate::drivers::vfs::FSResult<usize> {
        for (i, device) in DEVICE_MANAGER.lock().devices().iter().enumerate() {
            if Device::name(*device) == name {
                return Ok(i + 1);
            }
        }
        Err(super::FSError::NoSuchAFileOrDirectory)
    }
}

#[derive(Clone)]
pub struct DeviceInode {
    inodeid: usize,
}

impl DeviceInode {
    pub fn create(inodeid: usize) -> Inode {
        Arc::new(Mutex::new(Self { inodeid }))
    }

    pub fn device(&self) -> &'static dyn Device {
        DEVICE_MANAGER
            .lock()
            .get_device_at(self.inodeid - 1)
            .unwrap()
    }
}

impl InodeOps for Mutex<DeviceInode> {
    fn name(&self) -> String {
        Device::name(self.lock().device()).to_string()
    }

    fn inodeid(&self) -> usize {
        self.lock().inodeid
    }

    fn kind(&self) -> InodeType {
        InodeType::Device
    }

    fn read(&self, buffer: &mut [u8], offset: usize, count: usize) -> FSResult<usize> {
        self.lock().device().read(buffer, offset, count)
    }

    fn write(&self, buffer: &[u8], offset: usize) -> FSResult<usize> {
        self.lock().device().write(buffer, offset)
    }
}

pub struct DeviceFS {
    root_inode: Inode,
}

impl DeviceFS {
    pub fn new() -> Self {
        Self {
            root_inode: Arc::new(Mutex::new(DeviceManagerInode)),
        }
    }
}

impl FS for DeviceFS {
    fn name(&self) -> &'static str {
        "devices"
    }

    fn root_inode(&self) -> FSResult<Inode> {
        Ok(self.root_inode.clone())
    }

    fn get_inode(&self, inode_id: usize) -> FSResult<Option<Inode>> {
        if inode_id == 0 {
            return Ok(Some(self.root_inode.clone()));
        }

        for (i, _) in DEVICE_MANAGER.lock().devices().iter().enumerate() {
            if i == inode_id - 1 {
                return Ok(Some(DeviceInode::create(inode_id)));
            }
        }

        Ok(None)
    }

    fn open(&self, path: Path) -> FSResult<FileDescriptor> {
        let resolved = self.reslove_path(path)?;
        Ok(FileDescriptor::new(
            self as *const Self as *mut Self,
            resolved.clone(),
        ))
    }

    fn write(&self, file_descriptor: &mut FileDescriptor, buffer: &[u8]) -> FSResult<usize> {
        file_descriptor.node.write(buffer, 0)
    }

    fn read(&self, file_descriptor: &mut FileDescriptor, buffer: &mut [u8]) -> FSResult<usize> {
        file_descriptor.node.read(buffer, 0, 0)
    }

    fn diriter_open(&self, _fd: &mut FileDescriptor) -> FSResult<DirIter> {
        let length = DEVICE_MANAGER.lock().devices().len();

        let mut inodeids = Vec::with_capacity(length);
        for inodeid in 0..length {
            inodeids.push(inodeid + 1);
        }

        Ok(DirIter::new(
            self as *const DeviceFS as *mut DeviceFS,
            inodeids.into_boxed_slice(),
        ))
    }
}
