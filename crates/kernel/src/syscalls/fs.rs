use macros::syscall_handler;
use safa_abi::fs::{DirEntry, OpenOptions};

use crate::process::resources::Ri;
use crate::syscalls::{ErrorStatus, SyscallFFI};
use crate::{
    drivers::vfs::FSResult,
    fs::{self, FileRef},
    utils::path::Path,
};

/// Opens a file or directory with all permissions
#[syscall_handler]
fn sysopen_all(path: Path) -> FSResult<Ri> {
    FileRef::open_all(path).map(|ok| ok.ri())
}

/// Opens a file or directory with the specified options
#[syscall_handler]
fn sysopen(path: Path, options: u8) -> FSResult<Ri> {
    let options = OpenOptions::from_bits(options);
    FileRef::open_with_options(path, options).map(|ok| ok.ri())
}

/// Removes a path
#[syscall_handler]
fn sysremove_path(path: Path) -> FSResult<()> {
    fs::remove(path)
}

/// Creates a new file
#[syscall_handler]
fn syscreate(path: Path) -> FSResult<()> {
    fs::create(path)
}

/// Creates a new directory
#[syscall_handler]
fn syscreatedir(path: Path) -> FSResult<()> {
    fs::createdir(path)
}

#[syscall_handler]
fn sysget_direntry(path: Path, dest_direntry: &mut DirEntry) -> FSResult<()> {
    *dest_direntry = fs::get_direntry(path)?;
    Ok(())
}
