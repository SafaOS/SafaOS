use core::{fmt::Debug, mem::ManuallyDrop, ops::Deref};

use crate::threading::resources::{self, add_resource, Resource};

use super::{FSError, FSResult, FileDescriptor, FileSystem, Inode, InodeType, Path, VFS_STRUCT};

#[derive(Debug)]
/// A high-level wrapper around a file descriptor resource
/// that automatically closes the file descriptor when dropped
pub struct File(usize);

impl File {
    fn with_fd<T, R>(&self, then: T) -> R
    where
        T: FnOnce(&mut FileDescriptor) -> R,
    {
        unsafe {
            resources::get_resource(self.0, |mut resource| {
                let Resource::File(ref mut fd) = *resource else {
                    unreachable!()
                };

                then(fd)
            })
            .unwrap_unchecked()
        }
    }

    pub fn open(path: Path) -> FSResult<Self> {
        let fd = VFS_STRUCT
            .try_read()
            .ok_or(FSError::ResourceBusy)?
            .open(path)?;

        let fd_ri = add_resource(Resource::File(fd));
        Ok(Self(fd_ri))
    }

    pub fn read(&self, buffer: &mut [u8]) -> FSResult<usize> {
        self.with_fd(|fd| {
            VFS_STRUCT
                .try_read()
                .ok_or(FSError::ResourceBusy)?
                .read(fd, buffer)
        })
    }

    pub fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        self.with_fd(|fd| {
            VFS_STRUCT
                .try_read()
                .ok_or(FSError::ResourceBusy)?
                .write(fd, buffer)
        })
    }

    pub fn from_fd(fd: usize) -> Option<Self> {
        resources::get_resource(fd, |resource| {
            if let Resource::File(_) = *resource {
                Some(Self(fd))
            } else {
                None
            }
        })
        .flatten()
    }

    pub fn diriter_open(&self) -> FSResult<usize> {
        let diriter = self.with_fd(|fd| {
            VFS_STRUCT
                .try_read()
                .ok_or(FSError::ResourceBusy)?
                .open_diriter(fd)
        })?;

        Ok(resources::add_resource(Resource::DirIter(diriter)))
    }

    pub fn direntry(&self) -> DirEntry {
        let node = self.with_fd(|fd| fd.node.clone());
        DirEntry::get_from_inode(node)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        self.with_fd(|fd| fd.close());
        resources::remove_resource(self.0).unwrap();
    }
}

#[derive(Debug)]
/// A wrapper around a [`ManuallyDrop<File>`] which doesn't close the file descriptor when dropped
pub struct FileRef(ManuallyDrop<File>);

impl FileRef {
    pub fn open(path: Path) -> FSResult<Self> {
        let file = File::open(path)?;
        Ok(Self(ManuallyDrop::new(file)))
    }

    pub fn get(fd: usize) -> Option<Self> {
        Some(Self(ManuallyDrop::new(File::from_fd(fd)?)))
    }

    pub fn fd(&self) -> usize {
        self.0 .0
    }

    pub fn into_inner(self) -> File {
        ManuallyDrop::into_inner(self.0)
    }
}

impl Deref for FileRef {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[no_mangle]
pub fn create(path: Path) -> FSResult<()> {
    VFS_STRUCT
        .try_read()
        .ok_or(FSError::ResourceBusy)?
        .create(path)
}

#[no_mangle]
pub fn createdir(path: Path) -> FSResult<()> {
    VFS_STRUCT
        .try_read()
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

pub fn diriter_next(dir_ri: usize, direntry: &mut DirEntry) -> FSResult<()> {
    resources::get_resource(dir_ri, |mut resource| {
        if let Resource::DirIter(ref mut diriter) = *resource {
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
    resources::remove_resource(dir_ri).ok_or(FSError::InvaildFileDescriptorOrRes)
}
