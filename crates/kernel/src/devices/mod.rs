pub mod serial;
pub mod tty;

use crate::{
    arch::serial::SERIAL,
    debug,
    drivers::vfs::{self, CtlArgs, FSError, FSResult, VFS},
    terminal::FRAMEBUFFER_TERMINAL,
    time,
};

use crate::utils::locks::RwLock;
use safa_utils::{make_path, types::DriveName};

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
    let elapsed = time!(ms) - now;
    debug!(VFS, "Initialized devices in ({}ms) ...", elapsed);
}

pub trait Device: Send + Sync {
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

impl<T: CharDevice> Device for T {
    fn name(&self) -> &'static str {
        self.name()
    }
    fn read(&self, buffer: &mut [u8]) -> FSResult<usize> {
        self.read(buffer)
    }
    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        self.write(buffer)
    }
    fn ctl(&self, cmd: u16, args: CtlArgs) -> FSResult<()> {
        self.ctl(cmd, args)
    }
    fn sync(&self) -> FSResult<()> {
        self.sync()
    }
}
