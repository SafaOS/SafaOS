use super::ffi::SyscallFFI;
use crate::{
    drivers::vfs::{FSError, SeekOffset},
    process::resources::{self, ResourceData, Ri},
    utils::locks::Mutex,
};

use macros::syscall_handler;
use safa_abi::{
    errors::ErrorStatus,
    fs::{DirEntry, FileAttr},
};

#[syscall_handler]
fn syswrite(
    fd: Ri,
    offset: isize,
    buf: &[u8],
    dest_wrote: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let off = SeekOffset::from(offset);

    let wrote = resources::get_resource::<_, _, ErrorStatus>(fd, |r| match r.data() {
        ResourceData::File(fd) => Ok(fd.write(off, buf)?),
        ResourceData::ServerSocketConn(conn) => Ok(conn.write(buf)?),
        ResourceData::ClientSocketConn(conn) => Ok(conn.write(buf)?),
        _ => Err(ErrorStatus::UnsupportedResource),
    })?;

    if let Some(dest_wrote) = dest_wrote {
        *dest_wrote = wrote;
    }

    Ok(())
}

#[syscall_handler]
fn sysread(
    fd: Ri,
    offset: isize,
    buf: &mut [u8],
    dest_read: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let off = SeekOffset::from(offset);

    let bytes_read = resources::get_resource::<_, _, ErrorStatus>(fd, |r| match r.data() {
        ResourceData::File(fd) => Ok(fd.read(off, buf)?),
        ResourceData::ServerSocketConn(conn) => Ok(conn.read(buf)?),
        ResourceData::ClientSocketConn(conn) => Ok(conn.read(buf)?),
        _ => Err(ErrorStatus::UnsupportedResource),
    })?;

    if let Some(dest_read) = dest_read {
        *dest_read = bytes_read;
    }

    Ok(())
}

#[syscall_handler]
fn sysdiriter_open(dir_rd: Ri, dest_diriter: Option<&mut usize>) -> Result<(), ErrorStatus> {
    resources::get_resource(dir_rd, |resource| match resource.data() {
        ResourceData::File(fd) => {
            let diriter = fd.open_collection_iter()?;
            let ri = resources::add_global_resource(ResourceData::DirIter(Mutex::new(diriter)));
            if let Some(dest_diriter) = dest_diriter {
                *dest_diriter = ri;
            }
            Ok(())
        }
        _ => Err(ErrorStatus::UnsupportedResource),
    })
}

#[syscall_handler]
fn sysdiriter_next(diriter_rd: Ri, direntry: &mut DirEntry) -> Result<(), ErrorStatus> {
    resources::get_resource(diriter_rd, |resource| match resource.data() {
        ResourceData::DirIter(dir) => {
            let next = dir.lock().next();
            if let Some(next) = next {
                *direntry = next;
                Ok(())
            } else {
                *direntry = unsafe { core::mem::zeroed() };
                Err(ErrorStatus::Generic)
            }
        }
        _ => Err(ErrorStatus::UnsupportedResource),
    })
}

#[syscall_handler]
fn syssync(ri: Ri) -> Result<(), ErrorStatus> {
    resources::get_resource(ri, |resource| unsafe { resource.sync() })
}

#[syscall_handler]
fn systruncate(fd: Ri, len: usize) -> Result<(), ErrorStatus> {
    resources::get_resource(fd, |resource| match resource.data() {
        ResourceData::File(fd) => Ok(fd.truncate(len)?),
        _ => Err(FSError::UnsupportedResource),
    })
}

// TODO: add always successful syscall handlers support
#[syscall_handler]
fn sysfsize(ri: Ri, dest_fd: Option<&mut usize>) -> Result<(), ErrorStatus> {
    resources::get_resource(ri, |resource| match resource.data() {
        ResourceData::File(fd) => {
            if let Some(dest_fd) = dest_fd {
                *dest_fd = fd.size();
            }
            Ok(())
        }
        _ => Err(FSError::UnsupportedResource),
    })
}

#[syscall_handler]
fn sysattrs(ri: Ri, dest_attrs: Option<&mut FileAttr>) -> Result<(), ErrorStatus> {
    resources::get_resource(ri, |resource| match resource.data() {
        ResourceData::File(fd) => {
            if let Some(dest_attrs) = dest_attrs {
                *dest_attrs = fd.attrs();
            }
            Ok(())
        }
        _ => Err(FSError::UnsupportedResource),
    })
}

#[syscall_handler]
fn sysdup(resource: Ri, dest_resource: &mut Ri) -> Result<(), ErrorStatus> {
    *dest_resource = resources::duplicate_resource(resource)
        .map(|s| s.map_err(|()| ErrorStatus::ResourceCloneFailed))
        .ok_or(ErrorStatus::UnknownResource)
        .flatten()?;
    Ok(())
}

#[syscall_handler]
fn sysio_command(ri: Ri, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
    resources::get_resource(ri, |res| match res.data() {
        ResourceData::File(f) => f.send_command(cmd, arg),
        ResourceData::TrackedMapping(m) => m.send_command(cmd, arg),
        ResourceData::ServerSocketConn(conn) => conn.handle_command(cmd, arg),
        ResourceData::ClientSocketConn(conn) => conn.handle_command(cmd, arg),
        _ => Err(FSError::OperationNotSupported),
    })
}
