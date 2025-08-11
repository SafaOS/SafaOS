pub mod framebuffer;
pub mod serial;
pub mod tty;

use alloc::boxed::Box;

use crate::{
    arch::serial::SERIAL,
    debug,
    drivers::{
        framebuffer::FRAMEBUFFER_DRIVER,
        vfs::{self, FSError, FSResult, SeekOffset, VFS},
    },
    process::vas::MemMappedInterface,
    terminal::FRAMEBUFFER_TERMINAL,
    time,
};

use crate::utils::locks::RwLock;
use crate::utils::{path::make_path, types::DriveName};

pub fn add_device(vfs: &VFS, device: &'static dyn Device) {
    let path = make_path!("dev", device.name());
    vfs.mount_device(path, device).unwrap();
}

/// Mounts devices to the `dev:/` file system in the VFS
pub fn init(vfs: &mut VFS) {
    debug!(VFS, "Initializing devices ...");
    let now = time!(ms);
    vfs.mount(
        DriveName::new_const("dev"),
        RwLock::new(vfs::ramfs::RamFS::create()),
    )
    .expect("failed to mount `dev:/`");
    add_device(vfs, &*FRAMEBUFFER_TERMINAL);
    add_device(vfs, &*SERIAL);
    add_device(vfs, &*FRAMEBUFFER_DRIVER);
    let elapsed = time!(ms) - now;
    debug!(VFS, "Initialized devices in ({}ms) ...", elapsed);
}

pub trait Device: Send + Sync {
    fn name(&self) -> &'static str;
    fn read(&self, offset: SeekOffset, buffer: &mut [u8]) -> FSResult<usize>;
    fn write(&self, offset: SeekOffset, buffer: &[u8]) -> FSResult<usize>;
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

impl<T: CharDevice> Device for T {
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
