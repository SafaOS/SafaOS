use super::SyscallFFI;
use crate::{
    drivers::vfs::{
        self,
        expose::{DirEntry, DirIterRef, FileAttr, FileRef},
        CtlArgs, FSResult,
    },
    utils::{errors::ErrorStatus, path::Path},
};
use macros::syscall_handler;

#[syscall_handler]
fn sysopen(path: Path, dest_fd: Option<&mut usize>) -> FSResult<()> {
    let file_ref = FileRef::open(path)?;
    if let Some(dest_fd) = dest_fd {
        *dest_fd = file_ref.ri();
    }

    Ok(())
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
fn syscreate(path: Path) -> FSResult<()> {
    vfs::expose::create(path)
}

#[syscall_handler]
fn syscreatedir(path: Path) -> FSResult<()> {
    vfs::expose::createdir(path)
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
fn sysdiriter_next(
    diriter_rd: DirIterRef,
    direntry: &mut vfs::expose::DirEntry,
) -> Result<(), ErrorStatus> {
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
    *dest_direntry = vfs::expose::get_direntry(path)?;
    Ok(())
}

#[syscall_handler]
fn sysctl(fd: FileRef, cmd: u16, args: &[usize]) -> FSResult<()> {
    let ctl_args = CtlArgs::new(args);
    fd.ctl(cmd, ctl_args)
}
