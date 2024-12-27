// TODO: define write and read behaviour, especially write
pub mod expose;

use core::usize;

use crate::{
    debug, limine,
    threading::expose::getcwd,
    utils::{
        errors::{ErrorStatus, IntoErr},
        ustar::{self, TarArchiveIter},
    },
};
pub mod devicefs;
pub mod procfs;
pub mod ramfs;

use alloc::{
    borrow::ToOwned,
    boxed::Box,
    collections::btree_map::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use expose::DirEntry;
use lazy_static::lazy_static;
use spin::RwLock;
pub type Path<'a> = &'a str;

lazy_static! {
    pub static ref VFS_STRUCT: RwLock<VFS> = RwLock::new(VFS::new());
}

pub fn init() {
    debug!(VFS, "initing ...");
    let mut vfs = VFS_STRUCT.write();
    // ramfs
    let ramfs = Box::new(ramfs::RamFS::new());
    vfs.mount(b"ram", ramfs).unwrap();
    // devices
    vfs.mount(b"dev", Box::new(devicefs::DeviceFS::new()))
        .unwrap();
    // processes
    vfs.mount(b"proc", Box::new(procfs::ProcFS::new())).unwrap();
    // ramdisk
    let mut ramdisk = limine::get_ramdisk();
    let mut ramfs = Box::new(ramfs::RamFS::new());
    VFS::unpack_tar(&mut *ramfs, &mut ramdisk).expect("failed unpacking ramdisk archive");
    vfs.mount(b"sys", ramfs).expect("failed mounting");

    debug!(VFS, "done ...");
}

#[derive(Clone)]
pub struct FileDescriptor {
    pub mountpoint: *mut dyn FS,
    pub node: Inode,
    /// acts as a dir entry index for directories
    /// acts as a byte index for files
    pub read_pos: usize,
    /// acts as a byte index for files
    /// doesn't do anything for directories
    pub write_pos: usize,
}

impl FileDescriptor {
    pub fn new<'a>(mountpoint: *mut dyn FS, node: Inode) -> Self {
        Self {
            mountpoint,
            node,
            read_pos: 0,
            write_pos: 0,
        }
    }
}

#[derive(Debug, Clone)]
#[repr(u8)]
pub enum FSError {
    OperationNotSupported,
    NotAFile,
    NotADirectory,
    NoSuchAFileOrDirectory,
    InvaildDrive,
    InvaildPath,
    /// ethier a fd which points to a resource which isnt a FileDescriptor or it points to nothing
    InvaildFileDescriptorOrRes,
    AlreadyExists,
    NotExecuteable,
    ResourceBusy,
}

impl IntoErr for FSError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::OperationNotSupported => ErrorStatus::OperationNotSupported,
            Self::NotAFile => ErrorStatus::NotAFile,
            Self::NotADirectory => ErrorStatus::NotADirectory,
            Self::NoSuchAFileOrDirectory => ErrorStatus::NoSuchAFileOrDirectory,
            Self::InvaildPath => ErrorStatus::InvaildPath,
            Self::InvaildDrive => ErrorStatus::NoSuchAFileOrDirectory,
            Self::InvaildFileDescriptorOrRes => ErrorStatus::InvaildResource,
            Self::AlreadyExists => ErrorStatus::AlreadyExists,
            Self::NotExecuteable => ErrorStatus::NotExecutable,
            Self::ResourceBusy => ErrorStatus::Busy,
        }
    }
}
pub type FSResult<T> = Result<T, FSError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InodeType {
    File,
    Directory,
    Device,
}
pub trait InodeOps: Send + Sync {
    fn name(&self) -> String;
    /// gets an Inode from self
    fn get(&self, name: &str) -> FSResult<usize> {
        _ = name;
        FSResult::Err(FSError::OperationNotSupported)
    }
    /// checks if node contains `name` returns false if it doesn't or if it is not a directory
    fn contains(&self, name: &str) -> bool {
        _ = name;
        false
    }
    /// returns the size of node
    /// different nodes may use this differently but in case it is a normal file it will always give the
    /// file size in bytes
    fn size(&self) -> FSResult<usize> {
        Err(FSError::OperationNotSupported)
    }
    /// attempts to read `count` bytes of node data if it is a file
    /// panics if invaild `offset`
    /// returns the amount of bytes read
    fn read(&self, buffer: &mut [u8], offset: usize, count: usize) -> FSResult<usize> {
        _ = buffer;
        _ = offset;
        _ = count;
        Err(FSError::OperationNotSupported)
    }
    /// attempts to write `buffer.len` bytes from `buffer` into node data if it is a file starting
    /// from offset
    /// extends the nodes data and node size if `buffer.len` + `offset` is greater then node size
    /// returns the amount of bytes written
    fn write(&self, buffer: &[u8], offset: usize) -> FSResult<usize> {
        _ = buffer;
        _ = offset;
        Err(FSError::OperationNotSupported)
    }

    /// attempts to insert a node to self
    /// returns an FSError::NotADirectory if not a directory
    fn insert(&self, name: &str, node: usize) -> FSResult<()> {
        _ = name;
        _ = node;
        Err(FSError::OperationNotSupported)
    }

    fn truncate(&self, size: usize) -> FSResult<()> {
        _ = size;
        Err(FSError::OperationNotSupported)
    }

    fn inodeid(&self) -> usize;
    fn kind(&self) -> InodeType;

    #[inline(always)]
    fn is_dir(&self) -> bool {
        self.kind() == InodeType::Directory
    }

    fn open_diriter(&self, fs: *mut dyn FS) -> FSResult<DirIter> {
        _ = fs;
        Err(FSError::OperationNotSupported)
    }
}

/// unknown inode type
pub type Inode = Arc<dyn InodeOps>;
/// inode type with a known type
pub type InodeOf<T> = Arc<T>;

#[derive(Debug, Clone)]
pub struct DirIter {
    fs: *mut dyn FS,
    inode_ids: Box<[usize]>,
    index: usize,
}

impl DirIter {
    pub const fn new(fs: *mut dyn FS, inode_ids: Box<[usize]>) -> Self {
        Self {
            fs,
            inode_ids,
            index: 0,
        }
    }

    pub fn next(&mut self) -> Option<DirEntry> {
        let index = self.index;
        self.index += 1;

        if index >= self.inode_ids.len() {
            return None;
        }

        let inode_id = self.inode_ids[index];
        let inode = unsafe { (*self.fs).get_inode(inode_id) };

        match inode {
            Ok(Some(inode)) => Some(DirEntry::get_from_inode(inode)),
            Ok(None) => self.next(),
            _ => None,
        }
    }
}

pub trait FS: Send + Sync {
    /// returns the name of the fs
    /// for example, `TmpFS` name is "tmpfs"
    /// again we cannot use consts because of `dyn`...
    fn name(&self) -> &'static str;
    /// attempts to close a file cleanig all it's resources
    fn close(&self, file_descriptor: &mut FileDescriptor) -> FSResult<()> {
        _ = file_descriptor;
        Ok(())
    }

    fn get_inode(&self, inode_id: usize) -> FSResult<Option<Inode>> {
        _ = inode_id;
        Err(FSError::OperationNotSupported)
    }

    #[inline]
    fn root_inode(&self) -> FSResult<Inode> {
        Ok(self.get_inode(0)?.unwrap())
    }

    /// goes trough path to get the inode it refers to
    /// will err if there is no such a file or directory or path is straight up invaild
    fn reslove_path(&self, path: Path) -> FSResult<Inode> {
        let mut path = path.split(&['/', '\\']).peekable();

        let mut current_inode = self.root_inode()?;

        if path.peek() == Some(&"") {
            path.next();
        }

        // skips drive if it is provided
        if path.peek().is_some_and(|peek| peek.contains(':')) {
            path.next();
        }

        while let Some(depth) = path.next() {
            if depth == "" {
                if path.next() == None {
                    break;
                } else {
                    return Err(FSError::InvaildPath);
                }
            }

            if depth == "." {
                continue;
            }

            if !current_inode.is_dir() {
                return Err(FSError::NoSuchAFileOrDirectory);
            }

            if !current_inode.contains(depth) {
                return Err(FSError::NoSuchAFileOrDirectory);
            }

            let inodeid = current_inode.get(depth)?;
            current_inode = self.get_inode(inodeid)?.unwrap();
        }

        return Ok(current_inode.clone());
    }

    /// goes trough path to get the inode it refers to
    /// will err if there is no such a file or directory or path is straight up invaild
    /// assumes that the last depth in path is the filename and returns it alongside the parent dir
    fn reslove_path_uncreated<'a>(&self, path: Path<'a>) -> FSResult<(Inode, &'a str)> {
        let path = path.trim_end_matches('/');

        let (name, path) = {
            let beginning = path.bytes().rposition(|c| c == b'/');

            if let Some(idx) = beginning {
                (&path[idx + 1..], &path[..idx])
            } else {
                (path, "/")
            }
        };

        let resloved = self.reslove_path(path)?;
        if resloved.kind() != InodeType::Directory {
            return Err(FSError::NotADirectory);
        }

        Ok((resloved, name))
    }

    /// opens a path returning a file descriptor or an Err(()) if path doesn't exist
    fn open(&self, path: Path) -> FSResult<FileDescriptor> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }
    /// attempts to read `buffer.len` bytes from file_descriptor returns the actual count of the bytes read
    /// shouldn't read directories!
    fn read(&self, file_descriptor: &mut FileDescriptor, buffer: &mut [u8]) -> FSResult<usize> {
        _ = file_descriptor;
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }
    /// attempts to write `buffer.len` bytes to `file_descriptor`
    /// shouldn't write to directories!
    fn write(&self, file_descriptor: &mut FileDescriptor, buffer: &[u8]) -> FSResult<usize> {
        _ = file_descriptor;
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }
    /// creates an empty file named `name` in `path`
    fn create(&mut self, path: Path) -> FSResult<()> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }
    /// creates an empty dir named `name` in `path`
    fn createdir(&mut self, path: Path) -> FSResult<()> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }

    /// opens an iterator of directroy entires, fd must be a directory
    fn diriter_open(&self, fd: &mut FileDescriptor) -> FSResult<DirIter> {
        fd.node.open_diriter(fd.mountpoint)
    }
}

pub struct VFS {
    pub drivers: BTreeMap<Vec<u8>, Box<dyn FS>>,
}

impl VFS {
    pub fn new() -> Self {
        Self {
            drivers: BTreeMap::new(),
        }
    }
    /// mounts a file system as a drive
    /// returns Err(()) if not enough memory or there is an already mounted driver with that
    /// name
    pub fn mount(&mut self, name: &[u8], value: Box<dyn FS>) -> Result<(), ()> {
        let name = name.to_vec();

        if self.drivers.contains_key(&name) {
            Err(())
        } else {
            self.drivers.insert(name, value);
            Ok(())
        }
    }

    /// gets a drive from `self` named "`name`"
    /// or "`name`:" muttabily
    pub(self) fn get_with_name_mut(&mut self, name: &[u8]) -> Option<&mut Box<dyn FS>> {
        let mut name = name;

        if name.ends_with(b":") {
            name = &name[..name.len() - 1];
        }

        self.drivers.get_mut(name)
    }

    /// gets a drive from `self` named "`name`"
    /// or "`name`:" imuttabily
    pub(self) fn get_with_name(&self, name: &[u8]) -> Option<&Box<dyn FS>> {
        let mut name = name;

        if name.ends_with(b":") {
            name = &name[..name.len() - 1];
        }

        self.drivers.get(name)
    }
    /// gets the drive name from `path` then gets the drive
    /// path must be absolute starting with DRIVE_NAME:/
    /// also handles relative path
    pub(self) fn get_from_path_mut(&mut self, path: Path) -> FSResult<(&mut Box<dyn FS>, String)> {
        let mut spilt_path = path.split(&['/', '\\']);

        let drive = spilt_path.next().ok_or(FSError::InvaildDrive)?;
        let full_path = if !(drive.ends_with(':')) {
            &(getcwd().to_owned() + path)
        } else {
            path
        };

        return self.get_from_path_checked_mut(full_path);
    }

    /// gets the drive name from `path` then gets the drive
    /// path must be absolute starting with DRIVE_NAME:/
    /// also handles relative path
    pub(self) fn get_from_path(&self, path: Path) -> FSResult<(&Box<dyn FS>, String)> {
        let mut spilt_path = path.split(&['/', '\\']);

        let drive = spilt_path.next().ok_or(FSError::InvaildDrive)?;
        let full_path = if !(drive.ends_with(':')) {
            &(getcwd().to_owned() + path)
        } else {
            path
        };

        return self.get_from_path_checked(full_path);
    }

    /// get_from_path but path cannot be realtive to cwd
    pub(self) fn get_from_path_checked_mut(
        &mut self,
        path: Path,
    ) -> FSResult<(&mut Box<dyn FS>, String)> {
        let mut spilt_path = path.split(&['/', '\\']);

        let drive = spilt_path.next().ok_or(FSError::InvaildDrive)?;
        if !(drive.ends_with(':')) {
            return Err(FSError::InvaildDrive);
        }

        Ok((
            self.get_with_name_mut(drive.as_bytes())
                .ok_or(FSError::InvaildDrive)?,
            path.to_string(),
        ))
    }

    /// get_from_path but path cannot be realtive to cwd
    pub(self) fn get_from_path_checked(&self, path: Path) -> FSResult<(&Box<dyn FS>, String)> {
        let mut spilt_path = path.split(&['/', '\\']);

        let drive = spilt_path.next().ok_or(FSError::InvaildDrive)?;
        if !(drive.ends_with(':')) {
            return Err(FSError::InvaildDrive);
        }

        Ok((
            self.get_with_name(drive.as_bytes())
                .ok_or(FSError::InvaildDrive)?,
            path.to_string(),
        ))
    }

    /// checks if a path is a vaild dir returns Err if path has an error
    /// handles relative paths
    /// returns the absolute path if it is a dir
    pub fn verify_path_dir(&self, path: Path) -> FSResult<String> {
        let (mountpoint, path) = self.get_from_path(path)?;

        let res = mountpoint.reslove_path(&path)?;

        if !res.is_dir() {
            return Err(FSError::NotADirectory);
        }
        Ok(path)
    }

    pub fn unpack_tar(fs: &mut dyn FS, tar: &mut TarArchiveIter) -> FSResult<()> {
        while let Some(inode) = tar.next() {
            let path = inode.name();

            match inode.kind {
                ustar::Type::NORMAL => {
                    fs.create(path)?;

                    let mut opened = fs.open(path)?;
                    fs.write(&mut opened, inode.data())?;
                    fs.close(&mut opened)?;
                }

                ustar::Type::DIR => fs.createdir(path.trim_end_matches('/'))?,

                _ => return Err(FSError::OperationNotSupported),
            };
        }
        Ok(())
    }
}

impl FS for VFS {
    fn name(&self) -> &'static str {
        "vfs"
    }

    fn open(&self, path: Path) -> FSResult<FileDescriptor> {
        let (mountpoint, path) = self.get_from_path(path)?;

        let file = mountpoint.open(&path)?;

        Ok(file)
    }

    fn read(&self, file_descriptor: &mut FileDescriptor, buffer: &mut [u8]) -> FSResult<usize> {
        unsafe { (*file_descriptor.mountpoint).read(file_descriptor, buffer) }
    }

    fn write(&self, file_descriptor: &mut FileDescriptor, buffer: &[u8]) -> FSResult<usize> {
        unsafe { (*file_descriptor.mountpoint).write(file_descriptor, buffer) }
    }

    fn create(&mut self, path: Path) -> FSResult<()> {
        let (mountpoint, path) = self.get_from_path_mut(path)?;

        if path.ends_with('/') {
            return Err(FSError::NotAFile);
        }

        mountpoint.create(&path)
    }

    fn createdir(&mut self, path: Path) -> FSResult<()> {
        let (mountpoint, path) = self.get_from_path_mut(path)?;

        mountpoint.createdir(&path)
    }

    fn close(&self, file_descriptor: &mut FileDescriptor) -> FSResult<()> {
        unsafe { (*file_descriptor.mountpoint).close(file_descriptor) }
    }

    fn diriter_open(&self, fd: &mut FileDescriptor) -> FSResult<DirIter> {
        unsafe { (*fd.mountpoint).diriter_open(fd) }
    }
}
