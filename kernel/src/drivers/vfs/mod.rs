pub mod expose;

use core::fmt::Debug;

use crate::{
    debug,
    devices::{self, Device},
    limine,
    memory::{frame_allocator, paging::PAGE_SIZE},
    time,
    utils::{
        errors::{ErrorStatus, IntoErr},
        path::{CowPath, PathParts},
        ustar::{self, TarArchiveIter},
        HeaplessString,
    },
};
pub mod procfs;
pub mod ramfs;

use crate::utils::path::Path;
use alloc::{
    boxed::Box,
    collections::btree_map::{BTreeMap, Entry},
    string::{String, ToString},
    sync::Arc,
};
use expose::DirEntry;
use lazy_static::lazy_static;
use spin::{Mutex, RwLock};

pub type FileName = HeaplessString<{ DirEntry::MAX_NAME_LEN }>;

lazy_static! {
    pub static ref VFS_STRUCT: RwLock<VFS> = RwLock::new(VFS::create());
}

/// Defines a file descriptor resource
#[derive(Clone)]
pub struct FileDescriptor {
    mountpoint: Arc<dyn FileSystem>,
    node: Inode,
}

impl FileDescriptor {
    fn new(mountpoint: Arc<dyn FileSystem>, node: Inode) -> Self {
        Self { mountpoint, node }
    }

    pub fn close(&mut self) {
        self.node.close();
    }

    #[inline(always)]
    pub fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        self.node.read(offset, buffer)
    }

    #[inline(always)]
    pub fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        self.node.write(offset, buffer)
    }

    #[inline(always)]
    pub fn truncate(&self, len: usize) -> FSResult<()> {
        self.node.truncate(len)
    }

    #[inline(always)]
    pub fn sync(&self) -> FSResult<()> {
        self.node.sync()
    }

    #[inline(always)]
    pub fn open_diriter(&self) -> FSResult<DirIterDescriptor> {
        let inodes = self.node.open_diriter()?;
        let fs = self.mountpoint.clone();
        Ok(DirIterDescriptor::new(fs, inodes))
    }

    #[inline(always)]
    pub fn kind(&self) -> InodeType {
        self.node.kind()
    }

    #[inline(always)]
    pub fn ctl<'a>(&'a self, cmd: u16, args: CtlArgs<'a>) -> FSResult<()> {
        self.node.ctl(cmd, args)
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        self.node.size().unwrap_or(0)
    }
}

impl Drop for FileDescriptor {
    fn drop(&mut self) {
        self.close();
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
    InvaildOffset,
    InvaildName,
    /// Ctl
    InvaildCtlCmd,
    InvaildCtlArg,
    NotEnoughArguments,
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
            Self::InvaildOffset => ErrorStatus::InvaildOffset,
            Self::InvaildCtlCmd | Self::InvaildCtlArg => ErrorStatus::Generic,
            Self::InvaildName => ErrorStatus::StrTooLong,
            Self::NotEnoughArguments => ErrorStatus::NotEnoughArguments,
        }
    }
}
pub type FSResult<T> = Result<T, FSError>;

#[derive(Debug)]
pub struct CtlArgs<'a> {
    index: usize,
    args: &'a [usize],
}

pub trait CtlArg: Sized {
    fn try_from(value: usize) -> Option<Self>;
}

impl<T: TryFrom<usize>> CtlArg for T {
    fn try_from(value: usize) -> Option<Self> {
        TryFrom::try_from(value).ok()
    }
}

impl<'a> CtlArgs<'a> {
    pub fn new(args: &'a [usize]) -> Self {
        Self { index: 0, args }
    }

    pub fn get_ref_to<'b, T>(&mut self) -> FSResult<&'b mut T> {
        let it = self.get_ty::<usize>()? as *mut T;

        if it.is_null() || !it.is_aligned() {
            return Err(FSError::InvaildCtlArg);
        }
        Ok(unsafe { &mut *it })
    }

    pub fn get_ty<T: CtlArg>(&mut self) -> FSResult<T> {
        let it = self
            .args
            .get(self.index)
            .ok_or(FSError::NotEnoughArguments)?;
        self.index += 1;
        T::try_from(*it).ok_or(FSError::InvaildCtlArg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InodeType {
    File,
    Directory,
    Device,
}

pub trait InodeOps: Send + Sync {
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
    /// attempts to read `buffer.len` bytes of node data if it is a file
    /// returns the amount of bytes read
    /// offset in negative values acts the same as reading from at the end of the file + offset + 1
    fn read(&self, offset: isize, buffer: &mut [u8]) -> FSResult<usize> {
        _ = buffer;
        _ = offset;
        Err(FSError::OperationNotSupported)
    }
    /// attempts to write `buffer.len` bytes from `buffer` into node data if it is a file starting
    /// from offset
    /// extends the nodes data and node size if `buffer.len` + `offset` is greater then node size
    /// returns the amount of bytes written
    /// offset in negative values acts the same as writing to at the end of the file + offset + 1
    fn write(&self, offset: isize, buffer: &[u8]) -> FSResult<usize> {
        _ = buffer;
        _ = offset;
        Err(FSError::OperationNotSupported)
    }

    /// attempts to insert a node to self
    /// returns an FSError::NotADirectory if not a directory
    fn insert(&self, name: FileName, node: usize) -> FSResult<()> {
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

    fn open_diriter(&self) -> FSResult<Box<[DirIterInodeItem]>> {
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

    /// syncs the inode reads and writes
    fn sync(&self) -> FSResult<()> {
        Ok(())
    }

    fn ctl<'a>(&'a self, cmd: u16, args: CtlArgs<'a>) -> FSResult<()> {
        _ = cmd;
        _ = args;
        Err(FSError::OperationNotSupported)
    }
}

/// unknown inode type
pub type Inode = Arc<dyn InodeOps>;
/// inode type with a known type
pub type InodeOf<T> = Arc<T>;
pub type DirIterInodeItem = (HeaplessString<{ DirEntry::MAX_NAME_LEN }>, usize);

#[derive(Debug, Clone)]
pub struct DirIterDescriptor {
    fs: Arc<dyn FileSystem>,
    inodes: Box<[DirIterInodeItem]>,
    index: usize,
}

impl DirIterDescriptor {
    const fn new(fs: Arc<dyn FileSystem>, inodes: Box<[DirIterInodeItem]>) -> Self {
        Self {
            fs,
            inodes,
            index: 0,
        }
    }

    pub fn next(&mut self) -> Option<DirEntry> {
        let index = self.index;
        self.index += 1;

        if index >= self.inodes.len() {
            return None;
        }

        let (ref name, inode_id) = self.inodes[index];
        let inode = (*self.fs).get_inode(inode_id);

        match inode {
            Some(inode) => Some(DirEntry::get_from_inode(inode, name)),
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
    fn reslove_path(&self, path: PathParts) -> FSResult<Inode> {
        let mut current_inode = self.root_inode();
        if path.is_empty() {
            return Ok(current_inode);
        }

        for depth in path.iter() {
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
    fn reslove_path_uncreated<'a>(&self, path: PathParts<'a>) -> FSResult<(Inode, &'a str)> {
        let (name, path) = path.spilt_into_name();

        let name = name.ok_or(FSError::InvaildPath)?;
        let resloved = self.reslove_path(path)?;
        if resloved.kind() != InodeType::Directory {
            return Err(FSError::NotADirectory);
        }

        Ok((resloved, name))
    }

    /// creates an empty file named `name` in `path`
    fn create(&self, path: PathParts) -> FSResult<()> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }

    /// creates an empty dir named `name` in `path`
    fn createdir(&self, path: PathParts) -> FSResult<()> {
        _ = path;
        Err(FSError::OperationNotSupported)
    }

    /// mounts a device to `path`
    fn mount_device(&self, path: PathParts, device: &'static dyn Device) -> FSResult<()> {
        _ = path;
        _ = device;
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
    drives: BTreeMap<String, Arc<dyn FileSystem>>,
}

impl VFS {
    pub fn new() -> Self {
        Self {
            drives: BTreeMap::new(),
        }
    }

    /// Creates a new VFS and mounts the default filesystems
    pub fn create() -> Self {
        let mut this = Self::new();

        debug!(
            VFS,
            "Creating a new VFS with default initial filesystems ..."
        );

        let moment_memory_usage = frame_allocator::mapped_frames();
        let the_now = time!();

        // ramfs
        let ramfs = RwLock::new(ramfs::RamFS::new());
        this.mount("ram", ramfs).unwrap();
        // devices
        this.mount("dev", RwLock::new(ramfs::RamFS::new())).unwrap();
        devices::init(&this);
        // processes
        this.mount("proc", Mutex::new(procfs::ProcFS::create()))
            .unwrap();
        // ramdisk
        let mut ramdisk = limine::get_ramdisk();
        let mut ramfs = RwLock::new(ramfs::RamFS::new());

        debug!(VFS, "Unpacking ramdisk ...");
        this.unpack_tar(&mut ramfs, &mut ramdisk)
            .expect("failed unpacking ramdisk archive");
        debug!(VFS, "Mounting ramdisk ...");
        this.mount("sys", ramfs).expect("failed mounting");

        let elapsed = time!() - the_now;
        let used_memory = frame_allocator::mapped_frames() - moment_memory_usage;
        let total_memory_used = frame_allocator::mapped_frames();

        debug!(
            VFS,
            "done in ({}ms) ({}KiB mapped, {}KiB total) ...",
            elapsed,
            used_memory * PAGE_SIZE / 1024,
            total_memory_used * PAGE_SIZE / 1024
        );
        this
    }
    /// mounts a file system as a drive
    /// returns Err(()) if not enough memory or there is an already mounted driver with that
    /// name
    fn mount<F: FileSystem + 'static>(&mut self, name: &str, value: F) -> Result<(), ()> {
        if let Entry::Vacant(entry) = self.drives.entry(name.to_string()) {
            entry.insert(Arc::new(value));
            Ok(())
        } else {
            Err(())
        }
    }

    /// gets the drive name from `path` then gets the drive
    /// path must be absolute starting with DRIVE_NAME:/
    #[must_use]
    #[inline]
    fn get_from_path(&self, path: Path) -> FSResult<&Arc<dyn FileSystem>> {
        let drive = path
            .drive()
            .expect("drive was not put in the final absolute path using path::to_absolute_cwd");
        let drive = self.drives.get(drive).ok_or(FSError::InvaildDrive)?;
        Ok(drive)
    }
    /// gets the drive name from `path` then gets the drive
    /// path can be relative unlike [`Self::get_from_path`]
    /// returns the mountpoint and the relative path
    #[inline(always)]
    #[must_use = "it is bad to call this function without using the returned Absolute path"]
    fn get_from_path_relative<'a>(
        &self,
        path: Path<'a>,
    ) -> FSResult<(&Arc<dyn FileSystem>, CowPath<'a>)> {
        let path = path.to_absolute_cwd();
        let mountpoint = self.get_from_path(path.as_path())?;

        Ok((mountpoint, path))
    }

    /// checks if a path is a vaild dir returns Err if path has an error
    /// assumes that the path is absolute
    pub fn verify_path_dir(&self, path: Path) -> FSResult<()> {
        assert!(path.is_absolute());
        let mountpoint = self.get_from_path(path)?;
        let res = match path.parts() {
            Some(parts) => mountpoint.reslove_path(parts)?,
            // mountpoints are always directories
            None => return Ok(()),
        };

        if !res.is_dir() {
            return Err(FSError::NotADirectory);
        }
        Ok(())
    }

    fn unpack_tar(&self, fs: &mut dyn FileSystem, tar: &mut TarArchiveIter) -> FSResult<()> {
        while let Some(inode) = tar.next() {
            let path = PathParts::new(inode.name());
            if cfg!(debug_assertions) {
                debug!(VFS, "Unpacking ({}) {path} ...", inode.kind);
            }

            match inode.kind {
                ustar::Type::NORMAL => {
                    fs.create(path)?;

                    let node = fs.reslove_path(path)?;
                    node.write(0, inode.data())?;
                    node.close();
                }

                ustar::Type::DIR => fs.createdir(path)?,

                _ => return Err(FSError::OperationNotSupported),
            };
        }
        Ok(())
    }

    fn open(&self, path: Path) -> FSResult<FileDescriptor> {
        let (mountpoint, cow_path) = self.get_from_path_relative(path)?;
        let path = cow_path.as_path();

        mountpoint.on_open(path)?;
        let node = mountpoint.reslove_path(path.parts().unwrap_or_default())?;
        node.opened();
        Ok(FileDescriptor::new(mountpoint.clone(), node))
    }

    fn create_path(&self, path: Path) -> FSResult<()> {
        let (mountpoint, cow_path) = self.get_from_path_relative(path)?;
        let path = cow_path.as_path();
        mountpoint.create(path.parts().unwrap_or_default())
    }

    fn createdir(&self, path: Path) -> FSResult<()> {
        let (mountpoint, cow_path) = self.get_from_path_relative(path)?;
        let path = cow_path.as_path();
        mountpoint.createdir(path.parts().unwrap_or_default())
    }

    pub fn mount_device(&self, path: Path, device: &'static dyn Device) -> FSResult<()> {
        let (mountpoint, cow_path) = self.get_from_path_relative(path)?;
        let path = cow_path.as_path();
        mountpoint.mount_device(path.parts().unwrap_or_default(), device)
    }
}
