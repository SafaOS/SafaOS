use super::ffi::SyscallFFI;
use crate::{
    drivers::vfs::{CtlArgs, FSResult},
    fs::{self, DirIterRef, FileRef},
    utils::path::Path,
};

use macros::syscall_handler;
use safa_abi::{
    errors::ErrorStatus,
    fs::{DirEntry, FileAttr},
};

impl SyscallFFI for FileRef {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        FileRef::get(args).ok_or(ErrorStatus::InvalidResource)
    }
}

impl SyscallFFI for DirIterRef {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        DirIterRef::get(args).ok_or(ErrorStatus::InvalidResource)
    }
}

#[syscall_handler]
fn syswrite(
    fd: FileRef,
    offset: isize,
    buf: &[u8],
    dest_wrote: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let bytes_wrote = fd.write(offset, buf)?;
    if let Some(dest_wrote) = dest_wrote {
        *dest_wrote = bytes_wrote;
    }

    Ok(())
}

#[syscall_handler]
fn sysread(
    fd: FileRef,
    offset: isize,
    buf: &mut [u8],
    dest_read: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let bytes_read = fd.read(offset, buf)?;
    if let Some(dest_read) = dest_read {
        *dest_read = bytes_read;
    }

    Ok(())
}

#[syscall_handler]
fn sysdiriter_open(dir_rd: FileRef, dest_diriter: Option<&mut usize>) -> FSResult<()> {
    let diriter = dir_rd.diriter_open()?;
    if let Some(dest_diriter) = dest_diriter {
        *dest_diriter = diriter.ri();
    }
    Ok(())
}

#[syscall_handler]
fn sysdiriter_next(diriter_rd: DirIterRef, direntry: &mut DirEntry) -> Result<(), ErrorStatus> {
    let next = diriter_rd.next();
    if let Some(next) = next {
        *direntry = next;
        Ok(())
    } else {
        *direntry = unsafe { core::mem::zeroed() };
        Err(ErrorStatus::Generic)
    }
}

#[syscall_handler]
fn syssync(fd: FileRef) -> FSResult<()> {
    fd.sync()
}

#[syscall_handler]
fn systruncate(fd: FileRef, len: usize) -> FSResult<()> {
    fd.truncate(len)
}

// TODO: add always successful syscall handlers support
#[syscall_handler]
fn sysfsize(fd: FileRef, dest_fd: Option<&mut usize>) -> FSResult<()> {
    if let Some(dest_fd) = dest_fd {
        *dest_fd = fd.size();
    }
    Ok(())
}

#[syscall_handler]
fn sysattrs(fd: FileRef, dest_attrs: Option<&mut FileAttr>) -> FSResult<()> {
    if let Some(dest_attrs) = dest_attrs {
        *dest_attrs = fd.attrs();
    }
    Ok(())
}

#[syscall_handler]
fn sysdup(fd: FileRef, dest_fd: &mut FileRef) -> FSResult<()> {
    *dest_fd = fd.dup();
    Ok(())
}

#[syscall_handler]
fn sysget_direntry(path: Path, dest_direntry: &mut DirEntry) -> FSResult<()> {
    *dest_direntry = fs::get_direntry(path)?;
    Ok(())
}

#[syscall_handler]
fn sysctl(fd: FileRef, cmd: u16, args: &[usize]) -> FSResult<()> {
    let ctl_args = CtlArgs::new(args);
    fd.ctl(cmd, ctl_args)
}
