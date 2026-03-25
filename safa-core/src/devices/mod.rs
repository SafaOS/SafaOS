pub mod framebuffer;
/// Input Devices
pub mod input;
pub mod serial;
pub mod tty;

use alloc::boxed::Box;

use crate::{
    arch::serial::SERIAL,
    debug,
    devices::input::{keyboard::KEYBOARD_EVENT_QUEUE, mouse::MICE_EVENT_QUEUE},
    drivers::{
        framebuffer::FRAMEBUFFER_DRIVER,
        vfs::{self, FSError, FSResult, SeekOffset, VFS},
    },
    process::mem::MemMappedInterface,
    terminal::FRAMEBUFFER_TERMINAL,
    timer::{DurationFmt, SystemInstant},
    utils::path::{Path, PathParts},
};

use crate::utils::locks::RwLock;
use crate::utils::{path::make_path, types::DriveName};

pub fn add_static_device(vfs: &VFS, device: &'static dyn Device) {
    add_device(vfs, Box::new(StaticDevice(device)));
}

pub fn add_device(vfs: &VFS, device: Box<dyn Device>) {
    let path = make_path!("dev", device.name());
    vfs.mount_device(path, device).unwrap();
}

pub fn add_device_at(vfs: &VFS, device: Box<dyn Device>, subpath: &str) {
    let dir_path = make_path!("dev", subpath);
    vfs.createdir(dir_path).expect("Failed to create root dir");

    let dev_path = PathParts::new(device.name());

    let mut full_path = dir_path.into_owned().expect("Failed to convert into owned");
    full_path
        .append_simplified(unsafe { Path::from_raw_parts(None, Some(dev_path)) })
        .expect("Failed to append");

    vfs.mount_device(full_path.as_path(), device).unwrap();
}

/// Mounts devices to the `dev:/` file system in the VFS
pub fn init(vfs: &mut VFS) {
    debug!(VFS, "Initializing devices ...");
    let now = SystemInstant::now();
    vfs.mount(
        DriveName::new_const("dev"),
        RwLock::new(vfs::ramfs::RamFS::create()),
    )
    .expect("failed to mount `dev:/`");
    add_static_device(vfs, &*FRAMEBUFFER_TERMINAL);
    add_static_device(vfs, &SERIAL);
    add_static_device(vfs, &*FRAMEBUFFER_DRIVER);
    add_static_device(vfs, &KEYBOARD_EVENT_QUEUE);
    add_static_device(vfs, &MICE_EVENT_QUEUE);

    let elapsed = DurationFmt::new(now.elapsed());
    debug!(VFS, "Initialized devices in {} ...", elapsed);
}

/// A generic Device, can be a static Device where any interaction would apply to all open descriptors or a different interface for each descriptor
pub trait Device: Send + Sync {
    fn name(&self) -> &'static str;
    fn read(&self, offset: SeekOffset, buffer: &mut [u8]) -> FSResult<usize>;
    fn write(&self, offset: SeekOffset, buffer: &[u8]) -> FSResult<usize> {
        _ = offset;
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }
    fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        _ = cmd;
        _ = arg;
        Err(FSError::OperationNotSupported)
    }
    fn sync(&self) -> FSResult<()> {
        Ok(())
    }

    fn mmap(&self, offset: SeekOffset, page_count: usize) -> FSResult<Box<dyn MemMappedInterface>> {
        _ = offset;
        _ = page_count;
        Err(FSError::OperationNotSupported)
    }
}

pub trait CharDevice: Send + Sync {
    fn name(&self) -> &'static str;
    fn read(&self, buffer: &mut [u8]) -> FSResult<usize>;
    fn write(&self, buffer: &[u8]) -> FSResult<usize>;
    fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        _ = cmd;
        _ = arg;
        Err(FSError::OperationNotSupported)
    }
    fn sync(&self) -> FSResult<()> {
        Ok(())
    }
}

#[allow(unused)]
pub trait BlockDevice: Device {}

impl<T: CharDevice + ?Sized> Device for T {
    fn name(&self) -> &'static str {
        self.name()
    }
    fn read(&self, offset: SeekOffset, buffer: &mut [u8]) -> FSResult<usize> {
        _ = offset;
        self.read(buffer)
    }
    fn write(&self, offset: SeekOffset, buffer: &[u8]) -> FSResult<usize> {
        // offset is ignored in char devices
        _ = offset;
        self.write(buffer)
    }
    fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        self.send_command(cmd, arg)
    }
    fn sync(&self) -> FSResult<()> {
        self.sync()
    }
}

/// A device that is a wrapper over a static reference to a static device.
pub struct StaticDevice(pub &'static dyn Device);

impl Device for StaticDevice {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn mmap(&self, offset: SeekOffset, page_count: usize) -> FSResult<Box<dyn MemMappedInterface>> {
        self.0.mmap(offset, page_count)
    }
    fn read(&self, offset: SeekOffset, buffer: &mut [u8]) -> FSResult<usize> {
        self.0.read(offset, buffer)
    }
    fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        self.0.send_command(cmd, arg)
    }
    fn sync(&self) -> FSResult<()> {
        self.0.sync()
    }
    fn write(&self, offset: SeekOffset, buffer: &[u8]) -> FSResult<usize> {
        self.0.write(offset, buffer)
    }
}
