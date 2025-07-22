use core::{fmt::Debug, mem::ManuallyDrop, ops::Deref};

use safa_abi::raw::io::OpenOptions;

use crate::{
    drivers::vfs::SeekOffset,
    threading::resources::{self, Resource},
    utils::io::{IoError, Readable},
};

use super::{
    CollectionIterDescriptor, CtlArgs, FSError, FSObjectDescriptor, FSObjectType, FSResult, Path,
    VFS_STRUCT,
};

#[derive(Debug)]
/// A high-level wrapper around a file descriptor resource
/// that automatically closes the file descriptor when dropped
pub struct File(usize);

impl File {
    pub const fn fd(&self) -> usize {
        self.0
    }

    fn with_fd<T, R>(&self, then: T) -> R
    where
        T: FnOnce(&mut FSObjectDescriptor) -> R,
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
        let fd = VFS_STRUCT.read().open_all(path)?;

        let fd_ri = resources::add_resource(Resource::File(fd));
        Ok(Self(fd_ri))
    }

    pub fn open_with_options(path: Path, options: OpenOptions) -> FSResult<Self> {
        let fd = VFS_STRUCT.read().open(path, options)?;

        let fd_ri = resources::add_resource(Resource::File(fd));
        Ok(Self(fd_ri))
    }

    pub fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        let offset = if offset.is_negative() {
            SeekOffset::End((-offset) as usize)
        } else {
            SeekOffset::Start(offset as usize)
        };
        self.with_fd(|fd| fd.read(offset, buffer))
    }

    pub fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        let offset = if offset.is_negative() {
            SeekOffset::End((-offset) as usize)
        } else {
            SeekOffset::Start(offset as usize)
        };
        self.with_fd(|fd| fd.write(offset, buffer))
    }

    pub fn truncate(&self, len: usize) -> FSResult<()> {
        self.with_fd(|fd| fd.truncate(len))
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
        let diriter = self.with_fd(|fd| fd.open_collection_iter())?;

        Ok(DirIter(resources::add_resource(Resource::DirIter(diriter))))
    }

    pub fn sync(&self) -> FSResult<()> {
        self.with_fd(|fd| fd.sync())
    }

    pub fn kind(&self) -> FSObjectType {
        self.with_fd(|fd| fd.kind())
    }

    pub fn ctl<'a>(&'a self, cmd: u16, args: CtlArgs<'a>) -> FSResult<()> {
        self.with_fd(|fd| fd.ctl(cmd, args))
    }

    pub fn size(&self) -> usize {
        self.with_fd(|fd| fd.size())
    }

    pub fn attrs(&self) -> FileAttr {
        self.with_fd(|fd| fd.attrs())
    }

    pub fn dup(&self) -> Self {
        Self(resources::duplicate_resource(self.0))
    }
}

impl Drop for File {
    fn drop(&mut self) {
        resources::remove_resource(self.0).unwrap();
    }
}

impl Readable for File {
    fn read(&self, offset: isize, buf: &mut [u8]) -> Result<usize, IoError> {
        self.read(offset, buf).map_err(|e| match e {
            FSError::InvalidOffset => IoError::InvalidOffset,
            _ => IoError::Generic,
        })
    }
}

#[derive(Debug)]
/// A wrapper around a [`ManuallyDrop<File>`] which doesn't close the file descriptor when dropped
pub struct FileRef(ManuallyDrop<File>);

impl FileRef {
    pub fn dup(&self) -> Self {
        let file = self.0.dup();
        Self(ManuallyDrop::new(file))
    }

    pub fn open(path: Path) -> FSResult<Self> {
        let file = File::open(path)?;
        Ok(Self(ManuallyDrop::new(file)))
    }

    pub fn open_with_options(path: Path, options: OpenOptions) -> FSResult<Self> {
        let file = File::open_with_options(path, options)?;
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
        self.0.0
    }
}

impl Deref for FileRef {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[unsafe(no_mangle)]
pub fn create(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().createfile(path)
}

#[unsafe(no_mangle)]
pub fn remove(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().remove_path(path)
}

#[unsafe(no_mangle)]
pub fn createdir(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().createdir(path)
}

pub use safa_abi::raw::io::{DirEntry, FileAttr};

pub fn get_direntry(path: Path) -> FSResult<DirEntry> {
    VFS_STRUCT.read().get_direntry(path)
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
        F: FnOnce(&mut CollectionIterDescriptor) -> R,
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
        self.0.0
    }
}

impl Deref for DirIterRef {
    type Target = DirIter;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
