pub mod serial;
pub mod tty;

use crate::{
    arch::serial::SERIAL,
    debug,
    drivers::vfs::{CtlArgs, FSError, FSResult, InodeOps, VFS},
    terminal::FRAMEBUFFER_TERMINAL,
    time,
};

use safa_utils::make_path;

pub fn add_device(vfs: &VFS, device: &'static dyn Device) {
    let path = make_path!("dev", device.name());
    vfs.mount_device(path, device).unwrap();
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
    fn ctl(&self, cmd: u16, args: CtlArgs) -> FSResult<()> {
        _ = cmd;
        _ = args;
        Err(FSError::OperationNotSupported)
    }
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

    fn ctl(&self, cmd: u16, args: CtlArgs) -> FSResult<()> {
        CharDevice::ctl(self, cmd, args)
    }
}

impl<T: CharDevice> Device for T {
    fn name(&self) -> &'static str {
        self.name()
    }
}
