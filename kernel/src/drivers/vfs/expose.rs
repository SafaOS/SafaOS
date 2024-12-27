//! exposed functions of VFS they manually uses
//! a resource index instead of a file descriptor aka ri
use core::{fmt::Debug, usize};

use crate::threading::resources::{self, with_resource, Resource};

use super::{FSError, FSResult, FileDescriptor, Inode, InodeType, Path, FS, VFS_STRUCT};

/// gets a FileDescriptor from a fd (file_descriptor id) may return Err(FSError::InvaildFileDescriptor)
fn with_fd<T, R>(ri: usize, then: T) -> FSResult<R>
where
    T: FnOnce(&mut FileDescriptor) -> R,
{
    with_resource(ri, |resource| {
        if let Resource::File(ref mut fd) = resource {
            Ok(then(fd))
        } else {
            Err(FSError::NotAFile)
        }
    })
    .ok_or(FSError::InvaildFileDescriptorOrRes)?
}

#[no_mangle]
pub fn open(path: Path) -> FSResult<usize> {
    let fd = VFS_STRUCT
        .try_read()
        .ok_or(FSError::ResourceBusy)?
        .open(path)?;
    Ok(resources::add_resource(Resource::File(fd)))
}

#[no_mangle]
pub fn close(ri: usize) -> FSResult<()> {
    with_fd(ri, |fd| {
        VFS_STRUCT
            .try_read()
            .ok_or(FSError::ResourceBusy)?
            .close(fd)
    })??;

    _ = resources::remove_resource(ri);
    Ok(())
}

#[no_mangle]
pub fn read(ri: usize, buffer: &mut [u8]) -> FSResult<usize> {
    with_fd(ri, |fd| {
        VFS_STRUCT
            .try_read()
            .ok_or(FSError::ResourceBusy)?
            .read(fd, buffer)
    })?
}

#[no_mangle]
pub fn write(ri: usize, buffer: &[u8]) -> FSResult<usize> {
    with_fd(ri, |fd| {
        VFS_STRUCT
            .try_read()
            .ok_or(FSError::ResourceBusy)?
            .write(fd, buffer)
    })?
}

#[no_mangle]
pub fn create(path: Path) -> FSResult<()> {
    VFS_STRUCT
        .try_write()
        .ok_or(FSError::ResourceBusy)?
        .create(path)
}

#[no_mangle]
pub fn createdir(path: Path) -> FSResult<()> {
    VFS_STRUCT
        .try_write()
        .ok_or(FSError::ResourceBusy)?
        .createdir(path)
}

pub const MAX_NAME_LEN: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct DirEntry {
    pub kind: InodeType,
    pub size: usize,
    pub name_length: usize,
    pub name: [u8; 128],
}

impl DirEntry {
    pub fn get_from_inode(inode: Inode) -> Self {
        let name = inode.name();
        let name_slice = name.as_bytes();

        let kind = inode.kind();
        let size = inode.size().unwrap_or(0);

        let name_length = name_slice.len();
        let mut name = [0u8; MAX_NAME_LEN];

        name[..name_length].copy_from_slice(name_slice);

        Self {
            kind,
            size,
            name_length,
            name,
        }
    }

    pub const unsafe fn zeroed() -> Self {
        core::mem::zeroed()
    }
}

#[no_mangle]
/// opens a diriter as a resource
/// return the ri of the diriter
pub fn diriter_open(fd_ri: usize) -> FSResult<usize> {
    let diriter = with_fd(fd_ri, |fd| {
        VFS_STRUCT
            .try_read()
            .ok_or(FSError::ResourceBusy)?
            .diriter_open(fd)
    })??;

    Ok(resources::add_resource(Resource::DirIter(diriter)))
}

pub fn diriter_next(dir_ri: usize, direntry: &mut DirEntry) -> FSResult<()> {
    resources::with_resource(dir_ri, |resource| {
        if let Resource::DirIter(diriter) = resource {
            let next = diriter.next();
            if let Some(entry) = next {
                *direntry = entry.clone();
            } else {
                unsafe { *direntry = DirEntry::zeroed() }
            }
            Ok(())
        } else {
            Err(FSError::InvaildFileDescriptorOrRes)
        }
    })
    .ok_or(FSError::InvaildFileDescriptorOrRes)?
}

#[no_mangle]
/// may only Err if dir_ri is invaild
pub fn diriter_close(dir_ri: usize) -> FSResult<()> {
    resources::remove_resource(dir_ri).map_err(|_| FSError::InvaildFileDescriptorOrRes)
}

#[no_mangle]
pub fn fstat(ri: usize, direntry: &mut DirEntry) -> FSResult<()> {
    let node = with_fd(ri, |fd| fd.node.clone())?;
    *direntry = DirEntry::get_from_inode(node);
    Ok(())
}
