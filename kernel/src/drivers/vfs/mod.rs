pub mod expose;

use core::fmt::Debug;

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
    collections::btree_map::{BTreeMap, Entry},
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use expose::DirEntry;
use lazy_static::lazy_static;
use spin::{Mutex, RwLock};
pub type Path<'a> = &'a str;

lazy_static! {
    pub static ref VFS_STRUCT: RwLock<VFS> = RwLock::new(VFS::new());
}

pub fn init() {
    debug!(VFS, "initing ...");
    let mut vfs = VFS_STRUCT.write();
    // ramfs
    let ramfs = RwLock::new(ramfs::RamFS::new());
    vfs.mount(b"ram", ramfs).unwrap();
    // devices
    vfs.mount(b"dev", devicefs::DeviceFS::new()).unwrap();
    // processes
    vfs.mount(b"proc", Mutex::new(procfs::ProcFS::new()))
        .unwrap();
    // ramdisk
    let mut ramdisk = limine::get_ramdisk();
    let mut ramfs = RwLock::new(ramfs::RamFS::new());

    vfs.unpack_tar(&mut ramfs, &mut ramdisk)
        .expect("failed unpacking ramdisk archive");
    vfs.mount(b"sys", ramfs).expect("failed mounting");

    debug!(VFS, "done ...");
}

/// Defines a file descriptor resource
#[derive(Clone)]
pub struct FileDescriptor {
    mountpoint: Arc<dyn FileSystem>,
    pub node: Inode,
    /// acts as a dir entry index for directories
    /// acts as a byte index for files
    pub read_pos: usize,
    /// acts as a byte index for files
    /// doesn't do anything for directories
    pub write_pos: usize,
}

impl FileDescriptor {
    fn new(mountpoint: Arc<dyn FileSystem>, node: Inode) -> Self {
        Self {
            mountpoint,
            node,
            read_pos: 0,
            write_pos: 0,
        }
    }

    pub fn close(&mut self) {
        self.node.close();
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

    fn open_diriter(&self) -> FSResult<Box<[usize]>> {
        Err(FSError::OperationNotSupported)
    }

    /// executes when the inode is opened
    /// will be always called when the inode is opened, regardless of the file system
    fn opened(&self) {
        _ = self;
    }
    /// executes when the inode is closed
    /// will be always called when the inode is closed, regardless of the file system
    fn close(&self) {
        _ = self;
    }
}

/// unknown inode type
pub type Inode = Arc<dyn InodeOps>;
/// inode type with a known type
pub type InodeOf<T> = Arc<T>;

#[derive(Debug, Clone)]
pub struct DirIterDescriptor {
    fs: Arc<dyn FileSystem>,
    inode_ids: Box<[usize]>,
    index: usize,
}

impl DirIterDescriptor {
    const fn new(fs: Arc<dyn FileSystem>, inode_ids: Box<[usize]>) -> Self {
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
        let inode = (*self.fs).get_inode(inode_id);

        match inode {
            Some(inode) => Some(DirEntry::get_from_inode(inode)),
            None => self.next(),
        }
    }
}

pub trait FileSystem: Send + Sync {
    fn name(&self) -> &'static str;

    fn get_inode(&self, inode_id: usize) -> Option<Inode>;
    #[inline(always)]
    fn root_inode(&self) -> Inode {
        self.get_inode(0).unwrap()
    }

    /// called when a file is opened
    /// will be always called before the inode is opened, regardless of the file system
    fn on_open(&self, path: Path) -> FSResult<()> {
        _ = path;
        Ok(())
    }

    /// goes trough path to get the inode it refers to
    /// will err if there is no such a file or directory or path is straight up invaild
    fn reslove_path(&self, path: Path) -> FSResult<Inode> {
        let mut path = path.split(&['/', '\\']).peekable();

        let mut current_inode = self.root_inode();

        if path.peek() == Some(&"") {
            path.next();
        }

        // skips drive if it is provided
        if path.peek().is_some_and(|peek| peek.contains(':')) {
            path.next();
        }

        while let Some(depth) = path.next() {
            if depth.is_empty() {
                if path.next().is_none() {
                    break;
                } else {
                    return Err(FSError::InvaildPath);
                }
            }

            if depth == "." {
                continue;
            }

            if !current_inode.is_dir() {
                return Err(FSError::NotADirectory);
            }

            if !current_inode.contains(depth) {
                return Err(FSError::NoSuchAFileOrDirectory);
            }

            let inodeid = current_inode.get(depth)?;
            current_inode = self.get_inode(inodeid).unwrap();
        }

        Ok(current_inode.clone())
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

    /// attempts to read `buffer.len` bytes from file_descriptor returns the actual count of the bytes read
    /// shouldn't read directories!
    fn read(&self, file_descriptor: &mut FileDescriptor, buffer: &mut [u8]) -> FSResult<usize> {
        let count = buffer.len();
        let file_size = file_descriptor.node.size()?;

        let count = if file_descriptor.read_pos + count > file_size {
            file_size - file_descriptor.read_pos
        } else {
            count
        };

        file_descriptor
            .node
            .read(buffer, file_descriptor.read_pos, count)?;

        file_descriptor.read_pos += count;
        Ok(count)
    }
    /// attempts to write `buffer.len` bytes to `file_descriptor`
    /// shouldn't write to directories!
    fn write(&self, file_descriptor: &mut FileDescriptor, buffer: &[u8]) -> FSResult<usize> {
        if file_descriptor.write_pos == 0 {
            file_descriptor.node.truncate(0)?;
        }

        file_descriptor
            .node
            .write(buffer, file_descriptor.write_pos)?;

        file_descriptor.write_pos += buffer.len();

        Ok(buffer.len())
    }

    /// creates an empty file named `name` in `path`
    fn create(&self, path: Path) -> FSResult<()> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }

    /// creates an empty dir named `name` in `path`
    fn createdir(&self, path: Path) -> FSResult<()> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }
}

impl Debug for dyn FileSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl Debug for dyn InodeOps {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Inode").field(&self.inodeid()).finish()
    }
}
#[allow(clippy::upper_case_acronyms)]
pub struct VFS {
    drivers: BTreeMap<Vec<u8>, Arc<dyn FileSystem>>,
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
    fn mount<F: FileSystem + 'static>(&mut self, name: &[u8], value: F) -> Result<(), ()> {
        let name = name.to_vec();

        if let Entry::Vacant(entry) = self.drivers.entry(name) {
            entry.insert(Arc::new(value));
            Ok(())
        } else {
            Err(())
        }
    }

    /// gets a drive from `self` named "`name`"
    /// or "`name`:" imuttabily
    pub(self) fn get_with_name(&self, name: &[u8]) -> Option<&Arc<dyn FileSystem>> {
        let mut name = name;

        if name.ends_with(b":") {
            name = &name[..name.len() - 1];
        }

        self.drivers.get(name)
    }

    /// gets the drive name from `path` then gets the drive
    /// path must be absolute starting with DRIVE_NAME:/
    /// also handles relative path
    pub(self) fn get_from_path(&self, path: Path) -> FSResult<(&Arc<dyn FileSystem>, String)> {
        let mut spilt_path = path.split(&['/', '\\']);

        let drive = spilt_path.next().ok_or(FSError::InvaildDrive)?;
        let full_path = if !(drive.ends_with(':')) {
            &(getcwd().to_owned() + path)
        } else {
            path
        };

        self.get_from_path_checked(full_path)
    }

    /// get_from_path but path cannot be realtive to cwd
    pub(self) fn get_from_path_checked(
        &self,
        path: Path,
    ) -> FSResult<(&Arc<dyn FileSystem>, String)> {
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

    fn unpack_tar(&self, fs: &mut dyn FileSystem, tar: &mut TarArchiveIter) -> FSResult<()> {
        while let Some(inode) = tar.next() {
            let path = inode.name();

            match inode.kind {
                ustar::Type::NORMAL => {
                    fs.create(path)?;

                    let node = fs.reslove_path(path)?;
                    node.write(inode.data(), 0)?;
                    node.close();
                }

                ustar::Type::DIR => fs.createdir(path.trim_end_matches('/'))?,

                _ => return Err(FSError::OperationNotSupported),
            };
        }
        Ok(())
    }

    fn open(&self, path: Path) -> FSResult<FileDescriptor> {
        let (mountpoint, path) = self.get_from_path(path)?;
        mountpoint.on_open(&path)?;
        let node = mountpoint.reslove_path(&path)?;
        node.opened();
        Ok(FileDescriptor::new(mountpoint.clone(), node))
    }

    fn open_diriter(&self, file_descriptor: &mut FileDescriptor) -> FSResult<DirIterDescriptor> {
        let inodes = file_descriptor.node.open_diriter()?;
        let fs = file_descriptor.mountpoint.clone();

        Ok(DirIterDescriptor::new(fs, inodes))
    }
}

impl FileSystem for VFS {
    fn name(&self) -> &'static str {
        "vfs"
    }

    fn get_inode(&self, _: usize) -> Option<Inode> {
        unreachable!()
    }

    fn root_inode(&self) -> Inode {
        unreachable!()
    }

    fn reslove_path(&self, _: Path) -> FSResult<Inode> {
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn reslove_path_uncreated<'a>(&self, _: Path<'a>) -> FSResult<(Inode, &'a str)> {
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn on_open(&self, path: Path) -> FSResult<()> {
        _ = path;
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn read(&self, file_descriptor: &mut FileDescriptor, buffer: &mut [u8]) -> FSResult<usize> {
        file_descriptor
            .mountpoint
            .clone()
            .read(file_descriptor, buffer)
    }

    fn write(&self, file_descriptor: &mut FileDescriptor, buffer: &[u8]) -> FSResult<usize> {
        file_descriptor
            .mountpoint
            .clone()
            .write(file_descriptor, buffer)
    }

    fn create(&self, path: Path) -> FSResult<()> {
        let (mountpoint, path) = self.get_from_path(path)?;

        if path.ends_with('/') {
            return Err(FSError::NotAFile);
        }

        mountpoint.create(&path)
    }

    fn createdir(&self, path: Path) -> FSResult<()> {
        let (mountpoint, path) = self.get_from_path(path)?;

        mountpoint.createdir(&path)
    }
}
