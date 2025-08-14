use core::{mem::ManuallyDrop, ops::Deref};

use safa_abi::{
    errors::ErrorStatus,
    fs::{FSObjectType, OpenOptions},
};

use crate::{
    drivers::vfs::{FSError, FSObjectDescriptor, FSResult, SeekOffset, VFS_STRUCT},
    process::resources::{self, ResourceData},
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

    pub fn with_fd<T, R>(&self, then: T) -> R
    where
        T: FnOnce(&FSObjectDescriptor) -> R,
    {
        unsafe {
            resources::get_resource(self.0, |resource| {
                let ResourceData::File(fd) = resource.data() else {
                    unreachable!()
                };

                Ok::<_, ErrorStatus>(then(fd))
            })
            .unwrap_unchecked()
        }
    }

    /// Open a file at the given path with all permissions (read and write)
    pub fn open_all(path: Path) -> FSResult<Self> {
        let fd = VFS_STRUCT.read().open_all(path)?;

        let fd_ri = resources::add_global_resource(ResourceData::File(fd));
        Ok(Self(fd_ri))
    }

    /// Open a file at the given path with the specified options
    pub fn open_with_options(path: Path, options: OpenOptions) -> FSResult<Self> {
        let fd = VFS_STRUCT.read().open(path, options)?;

        let fd_ri = resources::add_global_resource(ResourceData::File(fd));
        Ok(Self(fd_ri))
    }

    /// Read data from the file at the given offset into the provided buffer
    pub fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        let offset = SeekOffset::from(offset);
        self.with_fd(|fd| fd.read(offset, buffer))
    }

    pub fn from_fd(fd: usize) -> Option<Result<Self, ()>> {
        resources::get_resource_reference(fd, |resource| {
            if let ResourceData::File(_) = resource.data() {
                Ok(Self(fd))
            } else {
                Err(())
            }
        })
    }

    /// Return the type of the file (Directory or Device or a normal File)
    pub fn kind(&self) -> FSObjectType {
        self.with_fd(|fd| fd.kind())
    }

    /// Duplicate the files resource into a new handle pointing to the same file
    pub fn dup(&self) -> Self {
        Self(
            resources::duplicate_resource(self.0)
                .expect("file doesn't point to anything")
                .expect("file doesn't point to file"),
        )
    }
}

impl Drop for File {
    fn drop(&mut self) {
        assert!(
            resources::remove_resource(self.0),
            "Dropping A File failed, invalid Resource ID"
        );
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
