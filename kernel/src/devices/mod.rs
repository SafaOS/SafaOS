pub mod serial;
pub mod tty;

use alloc::{
    collections::linked_list::LinkedList,
    string::{String, ToString},
};
use lazy_static::lazy_static;
use spin::Mutex;

use crate::{
    arch::serial::SERIAL,
    drivers::vfs::{FSResult, InodeOps},
    terminal::FRAMEBUFFER_TERMINAL,
};

pub struct DeviceManager {
    devices: LinkedList<&'static dyn Device>,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self {
            devices: LinkedList::new(),
        }
    }
    pub fn add_device(&mut self, device: &'static dyn Device) {
        self.devices.push_back(device);
    }

    pub fn devices(&self) -> &LinkedList<&'static dyn Device> {
        &self.devices
    }

    pub fn get_device_at(&self, index: usize) -> Option<&'static dyn Device> {
        for (i, device) in self.devices.iter().enumerate() {
            if i == index {
                return Some(*device);
            }
        }

        None
    }

    /// Create a new device manager and mounts all the initial devices
    pub fn create() -> Self {
        let mut this = Self::new();
        this.add_device(&*FRAMEBUFFER_TERMINAL);
        this.add_device(&*SERIAL);

        this
    }
}

pub trait Device: Send + Sync + InodeOps {
    fn name(&self) -> &'static str;
}

pub trait CharDevice: Send + Sync {
    fn name(&self) -> &'static str;
    fn read(&self, buffer: &mut [u8]) -> FSResult<usize>;
    fn write(&self, buffer: &[u8]) -> FSResult<usize>;
    fn sync(&self) -> FSResult<()> {
        Ok(())
    }
}

impl<T: CharDevice> InodeOps for T {
    fn name(&self) -> String {
        self.name().to_string()
    }

    fn kind(&self) -> crate::drivers::vfs::InodeType {
        crate::drivers::vfs::InodeType::Device
    }

    fn read(&self, _offset: isize, buffer: &mut [u8]) -> crate::drivers::vfs::FSResult<usize> {
        self.read(buffer)
    }

    fn write(&self, _offset: isize, buffer: &[u8]) -> crate::drivers::vfs::FSResult<usize> {
        CharDevice::write(self, buffer)
    }

    fn inodeid(&self) -> usize {
        0
    }

    fn sync(&self) -> crate::drivers::vfs::FSResult<()> {
        CharDevice::sync(self)
    }
}

impl<T: CharDevice> Device for T {
    fn name(&self) -> &'static str {
        self.name()
    }
}
lazy_static! {
    pub static ref DEVICE_MANAGER: Mutex<DeviceManager> = Mutex::new(DeviceManager::create());
}
