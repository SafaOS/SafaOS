use core::{mem::ManuallyDrop, ops::Deref};

use safa_abi::raw::io::{FSObjectType, FileAttr, OpenOptions};

use crate::{
    drivers::vfs::{CtlArgs, FSError, FSObjectDescriptor, FSResult, SeekOffset, VFS_STRUCT},
    fs::diriter::{DirIter, DirIterRef},
    scheduler::resources::{self, Resource},
    utils::{
        io::{IoError, Readable},
        path::Path,
    },
};

#[derive(Debug)]
/// A high-level wrapper around a file descriptor resource
/// that automatically closes the file descriptor when dropped
pub struct File(usize);

impl File {
    /// Returns the resource ID of the given file
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

    /// Open a file at the given path with all permissions (read and write)
    pub fn open_all(path: Path) -> FSResult<Self> {
        let fd = VFS_STRUCT.read().open_all(path)?;

        let fd_ri = resources::add_resource(Resource::File(fd));
        Ok(Self(fd_ri))
    }

    /// Open a file at the given path with the specified options
    pub fn open_with_options(path: Path, options: OpenOptions) -> FSResult<Self> {
        let fd = VFS_STRUCT.read().open(path, options)?;

        let fd_ri = resources::add_resource(Resource::File(fd));
        Ok(Self(fd_ri))
    }

    /// Read data from the file at the given offset into the provided buffer
    pub fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        let offset = if offset.is_negative() {
            SeekOffset::End((-offset) as usize)
        } else {
            SeekOffset::Start(offset as usize)
        };
        self.with_fd(|fd| fd.read(offset, buffer))
    }

    /// Write data to the file at the given offset from the provided buffer
    pub fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        let offset = if offset.is_negative() {
            SeekOffset::End((-offset) as usize)
        } else {
            SeekOffset::Start(offset as usize)
        };
        self.with_fd(|fd| fd.write(offset, buffer))
    }

    /// Truncate the file to the given length
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

    /// Open a directory iterator for the given file (assuming it is a directory otherwise returns an error), closes the resource when dropped unlike [`FileRef::diriter_open`]
    pub fn diriter_open(&self) -> FSResult<DirIter> {
        let diriter = self.with_fd(|fd| fd.open_collection_iter())?;

        Ok(DirIter(resources::add_resource(Resource::DirIter(diriter))))
    }

    /// Sync the file
    pub fn sync(&self) -> FSResult<()> {
        self.with_fd(|fd| fd.sync())
    }

    /// Return the type of the file (Directory or Device or a normal File)
    pub fn kind(&self) -> FSObjectType {
        self.with_fd(|fd| fd.kind())
    }

    /// Performs a `ctl` operation on the given file (assuming it is a device)
    pub fn ctl<'a>(&'a self, cmd: u16, args: CtlArgs<'a>) -> FSResult<()> {
        self.with_fd(|fd| fd.ctl(cmd, args))
    }

    /// Return the size of the file
    ///
    /// size is undefined for directories and devices
    pub fn size(&self) -> usize {
        self.with_fd(|fd| fd.size())
    }

    pub fn attrs(&self) -> FileAttr {
        self.with_fd(|fd| fd.attrs())
    }

    /// Duplicate the files resource into a new handle pointing to the same file
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
    /// Duplicate the files resource into a new handle pointing to the same file
    pub fn dup(&self) -> Self {
        let file = self.0.dup();
        Self(ManuallyDrop::new(file))
    }

    /// Open a file at the given path with all permissions (read and write), returning a new [`FileRef`] instance that isn't closed when dropped
    pub fn open_all(path: Path) -> FSResult<Self> {
        let file = File::open_all(path)?;
        Ok(Self(ManuallyDrop::new(file)))
    }

    /// Open a file at the given path with the specified permissions, returning a new [`FileRef`] instance that isn't closed when dropped
    pub fn open_with_options(path: Path, options: OpenOptions) -> FSResult<Self> {
        let file = File::open_with_options(path, options)?;
        Ok(Self(ManuallyDrop::new(file)))
    }

    /// Open a directory iterator for the given file (assuming it is a directory otherwise returns an error), doesn't close the resource when dropped unlike [`File::diriter_open`]
    pub fn diriter_open(&self) -> FSResult<DirIterRef> {
        self.0
            .diriter_open()
            .map(|x| DirIterRef(ManuallyDrop::new(x)))
    }

    pub fn get(fd: usize) -> Option<Self> {
        Some(Self(ManuallyDrop::new(File::from_fd(fd)?)))
    }

    /// Return the resource ID (equivalent to file descriptor in linux) of the file
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
