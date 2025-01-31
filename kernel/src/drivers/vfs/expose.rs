use core::{fmt::Debug, mem::ManuallyDrop, ops::Deref};

use crate::threading::resources::{self, Resource};

use super::{
    DirIterDescriptor, FSResult, FileDescriptor, FileSystem, Inode, InodeType, Path, VFS_STRUCT,
};

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
        let fd = VFS_STRUCT.read().open(path)?;

        let fd_ri = resources::add_resource(Resource::File(fd));
        Ok(Self(fd_ri))
    }

    pub fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        self.with_fd(|fd| VFS_STRUCT.read().read(fd, offset, buffer))
    }

    pub fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        self.with_fd(|fd| VFS_STRUCT.read().write(fd, offset, buffer))
    }

    pub fn truncate(&self, len: usize) -> FSResult<()> {
        // TODO: work more on truncating, for now we are using the node directly
        // i am not really sure if the VFS layer is even needed anymore because even reads and writes are just directing to the node
        self.with_fd(|fd| fd.node.truncate(len))
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

    pub fn diriter_open(&self) -> FSResult<DirIter> {
        let diriter = self.with_fd(|fd| VFS_STRUCT.read().open_diriter(fd))?;

        Ok(DirIter(resources::add_resource(Resource::DirIter(diriter))))
    }

    pub fn direntry(&self) -> DirEntry {
        let node = self.with_fd(|fd| fd.node.clone());
        DirEntry::get_from_inode(node)
    }

    pub fn sync(&self) -> FSResult<()> {
        self.with_fd(|fd| fd.node.sync())
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

    pub fn diriter_open(&self) -> FSResult<DirIterRef> {
        self.0
            .diriter_open()
            .map(|x| DirIterRef(ManuallyDrop::new(x)))
    }

    pub fn get(fd: usize) -> Option<Self> {
        Some(Self(ManuallyDrop::new(File::from_fd(fd)?)))
    }

    pub fn ri(&self) -> usize {
        self.0 .0
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
    VFS_STRUCT.read().create(path)
}

#[no_mangle]
pub fn createdir(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().createdir(path)
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

/// a wrapper around a DirIterDescriptor resource which closes the diriter when dropped
pub struct DirIter(usize);

impl DirIter {
    pub fn from_ri(ri: usize) -> Option<Self> {
        resources::get_resource(ri, |resource| {
            if let Resource::DirIter(_) = *resource {
                Some(Self(ri))
            } else {
                None
            }
        })
        .flatten()
    }

    fn with_diriter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DirIterDescriptor) -> R,
    {
        unsafe {
            resources::get_resource(self.0, |mut resource| {
                let Resource::DirIter(ref mut diriter) = *resource else {
                    unreachable!()
                };

                f(diriter)
            })
            .unwrap_unchecked()
        }
    }

    pub fn next(&self) -> Option<DirEntry> {
        self.with_diriter(|diriter| diriter.next())
    }
}

impl Drop for DirIter {
    fn drop(&mut self) {
        resources::remove_resource(self.0).unwrap();
    }
}

/// a wrapper around [`ManuallyDrop<DirIter>`] which doesn't close the diriter when dropped
pub struct DirIterRef(ManuallyDrop<DirIter>);

impl DirIterRef {
    pub fn get(ri: usize) -> Option<Self> {
        let diriter = DirIter::from_ri(ri)?;
        Some(Self(ManuallyDrop::new(diriter)))
    }

    pub fn ri(&self) -> usize {
        self.0 .0
    }
}

impl Deref for DirIterRef {
    type Target = DirIter;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
