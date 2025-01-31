use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use crate::devices::{Device, DEVICE_MANAGER};

use super::{FSResult, FileSystem, Inode, InodeOps, InodeType};

#[derive(Clone)]
pub enum DeviceInode {
    RootInode,
    Device(usize),
}

impl DeviceInode {
    pub fn create_device(index: usize) -> Arc<Self> {
        Arc::new(Self::Device(index))
    }

    pub fn device(&self) -> Option<&'static dyn Device> {
        match self {
            Self::Device(index) => Some(DEVICE_MANAGER.lock().get_device_at(*index).unwrap()),
            _ => None,
        }
    }
}

impl InodeOps for DeviceInode {
    fn name(&self) -> String {
        match self.device() {
            Some(device) => Device::name(device).to_string(),
            None => "".to_string(),
        }
    }

    fn inodeid(&self) -> usize {
        match self.device() {
            Some(device) => device.inodeid(),
            None => 0,
        }
    }

    fn kind(&self) -> InodeType {
        match self {
            Self::RootInode => InodeType::Directory,
            Self::Device(_) => InodeType::Device,
        }
    }

    fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        match self.device() {
            Some(device) => device.read(offset, buffer),
            None => FSResult::Err(super::FSError::NotAFile),
        }
    }

    fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        match self.device() {
            Some(device) => device.write(offset, buffer),
            None => FSResult::Err(super::FSError::NotAFile),
        }
    }

    fn open_diriter(&self) -> FSResult<alloc::boxed::Box<[usize]>> {
        match self.device() {
            Some(_) => FSResult::Err(super::FSError::NotADirectory),
            None => {
                let mut devices = Vec::with_capacity(DEVICE_MANAGER.lock().devices().len());
                for (i, _) in DEVICE_MANAGER.lock().devices().iter().enumerate() {
                    devices.push(i);
                }

                Ok(devices.into_boxed_slice())
            }
        }
    }

    fn contains(&self, name: &str) -> bool {
        match self.device() {
            Some(_) => false,
            None => {
                for device in DEVICE_MANAGER.lock().devices().iter() {
                    if Device::name(*device) == name {
                        return true;
                    }
                }

                false
            }
        }
    }

    fn get(&self, name: &str) -> FSResult<usize> {
        match self.device() {
            Some(_) => Err(super::FSError::NotADirectory),
            None => {
                for (i, device) in DEVICE_MANAGER.lock().devices().iter().enumerate() {
                    if Device::name(*device) == name {
                        return Ok(i + 1);
                    }
                }
                Err(super::FSError::NoSuchAFileOrDirectory)
            }
        }
    }
    fn sync(&self) -> FSResult<()> {
        match self.device() {
            Some(device) => device.sync(),
            None => Err(super::FSError::NotAFile),
        }
    }
}

pub struct DeviceFS {
    root_inode: Arc<DeviceInode>,
}

impl DeviceFS {
    pub fn new() -> Self {
        Self {
            root_inode: Arc::new(DeviceInode::RootInode),
        }
    }
}

impl FileSystem for DeviceFS {
    fn name(&self) -> &'static str {
        "DevFS"
    }

    fn get_inode(&self, inode_id: usize) -> Option<Inode> {
        if inode_id == 0 {
            return Some(self.root_inode.clone());
        }

        if inode_id - 1 < DEVICE_MANAGER.lock().devices().len() {
            return Some(DeviceInode::create_device(inode_id - 1));
        }
        None
    }
}
