pub mod serial;
pub mod tty;

use alloc::format;

use crate::{
    arch::serial::SERIAL,
    debug,
    drivers::vfs::{FSResult, FileSystem, InodeOps, VFS},
    terminal::FRAMEBUFFER_TERMINAL,
    time,
};

pub fn add_device(vfs: &VFS, device: &'static dyn Device) {
    let path = format!("dev:/{}", Device::name(device));
    vfs.mount_device(&path, device).unwrap();
}

pub fn init(vfs: &VFS) {
    debug!(VFS, "Initializing devices ...");
    let now = time!();
    add_device(vfs, &*FRAMEBUFFER_TERMINAL);
    add_device(vfs, &*SERIAL);
    let elapsed = time!() - now;
    debug!(VFS, "Initialized devices in ({}ms) ...", elapsed);
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
