use macros::syscall_handler;
use safa_abi::fs::{DirEntry, OpenOptions};

use crate::syscalls::{ErrorStatus, SyscallFFI};
use crate::{
    drivers::vfs::FSResult,
    fs::{self, FileRef},
    utils::path::Path,
};

/// Opens a file or directory with all permissions
#[syscall_handler]
fn sysopen_all(path: Path, dest_fd: Option<&mut usize>) -> FSResult<()> {
    let file_ref = FileRef::open_all(path)?;
    if let Some(dest_fd) = dest_fd {
        *dest_fd = file_ref.ri();
    }

    Ok(())
}

/// Opens a file or directory with the specified options
#[syscall_handler]
fn sysopen(path: Path, options: u8, dest_fd: Option<&mut usize>) -> FSResult<()> {
    let options = OpenOptions::from_bits(options);
    let file_ref = FileRef::open_with_options(path, options)?;

    if let Some(dest_fd) = dest_fd {
        *dest_fd = file_ref.ri();
    }

    Ok(())
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
