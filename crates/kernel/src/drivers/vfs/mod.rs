// TODO: write some tests?
pub mod expose;

use core::fmt::{Debug, Display};

use crate::{
    debug,
    devices::{self, Device},
    limine,
    memory::{frame_allocator, paging::PAGE_SIZE},
    threading::this_state,
    time,
    utils::{
        errors::{ErrorStatus, IntoErr},
        path::PathParts,
        ustar::{self, TarArchiveIter},
    },
};

pub mod procfs;
pub mod ramfs;
#[cfg(test)]
pub mod tests;

use crate::utils::locks::{Mutex, RwLock};
use crate::utils::path::Path;
use crate::utils::types::Name;
use alloc::{
    boxed::Box,
    collections::btree_map::{BTreeMap, Entry},
    sync::Arc,
};
use expose::{DirEntry, FileAttr};
use lazy_static::lazy_static;
use safa_utils::{
    path::PathError,
    types::{DriveName, FileName},
};

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
        _ = self.node.sync();
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

    #[inline(always)]
    pub fn attrs(&self) -> FileAttr {
        FileAttr::from_inode(&self.node)
    }
}

impl Drop for FileDescriptor {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[repr(u8)]
pub enum FSError {
    InvalidResource,
    OperationNotSupported,
    NotAFile,
    NotADirectory,
    NoSuchAFileOrDirectory,
    InvalidDrive,
    InvalidPath,
    PathTooLong,
    AlreadyExists,
    NotExecutable,
    InvalidOffset,
    InvalidName,
    /// Ctl
    InvalidCtlCmd,
    InvalidCtlArg,
    NotEnoughArguments,
}

impl Display for FSError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl IntoErr for FSError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::OperationNotSupported => ErrorStatus::OperationNotSupported,
            Self::NotAFile => ErrorStatus::NotAFile,
            Self::NotADirectory => ErrorStatus::NotADirectory,
            Self::NoSuchAFileOrDirectory => ErrorStatus::NoSuchAFileOrDirectory,
            Self::InvalidPath => ErrorStatus::InvalidPath,
            Self::InvalidDrive => ErrorStatus::NoSuchAFileOrDirectory,
            Self::AlreadyExists => ErrorStatus::AlreadyExists,
            Self::NotExecutable => ErrorStatus::NotExecutable,
            Self::InvalidOffset => ErrorStatus::InvalidOffset,
            Self::InvalidCtlCmd | Self::InvalidCtlArg => ErrorStatus::Generic,
            Self::InvalidName | Self::PathTooLong => ErrorStatus::StrTooLong,
            Self::NotEnoughArguments => ErrorStatus::NotEnoughArguments,
            Self::InvalidResource => ErrorStatus::InvalidResource,
        }
    }
}

impl From<PathError> for FSError {
    fn from(value: PathError) -> Self {
        match value {
            PathError::DriveNameTooLong => Self::InvalidDrive,
            PathError::PathPartsTooLong => Self::PathTooLong,
            PathError::FailedToJoinPaths | PathError::InvalidPath => Self::InvalidPath,
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
            return Err(FSError::InvalidCtlArg);
        }
        Ok(unsafe { &mut *it })
    }

    pub fn get_ty<T: CtlArg>(&mut self) -> FSResult<T> {
        let it = self
            .args
            .get(self.index)
            .ok_or(FSError::NotEnoughArguments)?;
        self.index += 1;
        T::try_from(*it).ok_or(FSError::InvalidCtlArg)
    }
}

// InodeType implementition
pub use safa_utils::abi::raw::io::InodeType;
use thiserror::Error;

pub trait InodeOps: Send + Sync {
    /// gets an Inode from self
    fn get(&self, name: &str) -> FSResult<usize> {
        _ = name;
        FSResult::Err(FSError::OperationNotSupported)
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
    fn insert(&self, name: Name, node: usize) -> FSResult<()> {
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
pub type DirIterInodeItem = (FileName, usize);

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
    /// will err if there is no such a file or directory or path is straight up invalid
    fn resolve_pathparts(&self, path: PathParts, root_node: Inode) -> FSResult<Inode> {
        let mut current_inode = root_node;
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

            let inodeid = current_inode.get(depth)?;
            current_inode = self.get_inode(inodeid).unwrap();
        }

        Ok(current_inode)
    }

    /// creates an empty file named `name` relative to Inode
    fn create(&self, node: Inode, name: &str) -> FSResult<()> {
        _ = node;
        _ = name;
        Err(FSError::OperationNotSupported)
    }

    /// creates an empty dir named `name` relative to Inode
    fn createdir(&self, node: Inode, name: &str) -> FSResult<()> {
        _ = node;
        _ = name;
        Err(FSError::OperationNotSupported)
    }

    /// mounts a device as `name` relative to `node`
    fn mount_device(&self, node: Inode, name: &str, device: &'static dyn Device) -> FSResult<()> {
        _ = device;
        _ = node;
        _ = name;
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
    drives: BTreeMap<DriveName, Arc<dyn FileSystem>>,
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
        let the_now = time!(ms);
        // temporary directory
        let tempfs = RwLock::new(ramfs::RamFS::new());
        this.mount(DriveName::new_const("tmp"), tempfs).unwrap();
        // ramfs
        let ramfs = RwLock::new(ramfs::RamFS::new());
        this.mount(DriveName::new_const("ram"), ramfs).unwrap();

        devices::init(&mut this);
        // processes
        this.mount(
            DriveName::new_const("proc"),
            Mutex::new(procfs::ProcFS::create()),
        )
        .unwrap();
        // ramdisk
        let mut ramdisk = limine::get_ramdisk();
        let mut ramfs = RwLock::new(ramfs::RamFS::new());

        debug!(VFS, "Unpacking ramdisk ...");
        this.unpack_tar(&mut ramfs, &mut ramdisk)
            .expect("failed unpacking ramdisk archive");
        debug!(VFS, "Mounting ramdisk ...");
        this.mount(DriveName::new_const("sys"), ramfs)
            .expect("failed mounting");

        let elapsed = time!(ms) - the_now;
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
    pub fn mount<F: FileSystem + 'static>(&mut self, name: DriveName, value: F) -> Result<(), ()> {
        if let Entry::Vacant(entry) = self.drives.entry(name) {
            entry.insert(Arc::new(value));
            Ok(())
        } else {
            Err(())
        }
    }

    /// gets the drive name from `path` then gets the drive
    /// path must be absolute, or else it'd panic
    #[must_use]
    #[inline]
    fn get_from_path(&self, path: Path) -> FSResult<&Arc<dyn FileSystem>> {
        let drive = path.drive().expect("path is not absolute");
        let drive = self.drives.get(drive).ok_or(FSError::InvalidDrive)?;
        Ok(drive)
    }

    #[must_use]
    #[inline]
    fn resolve_abs_path(&self, path: Path) -> FSResult<(&Arc<dyn FileSystem>, Inode)> {
        let drive = self.get_from_path(path)?;
        let drive_root = drive.root_inode();
        let Some(parts) = path.parts() else {
            // empty paths always point to drive_root, little optimization
            return Ok((drive, drive_root));
        };

        let resolved = drive.resolve_pathparts(parts, drive_root)?;
        Ok((drive, resolved))
    }

    #[must_use]
    #[inline]
    /// Tries to resolve pathparts relative to `cwd_path` to a Drive and an Inode
    fn resolve_relative_path(
        &self,
        cwd_path: Path,
        relative: PathParts,
    ) -> FSResult<(&Arc<dyn FileSystem>, Inode)> {
        let (drive, cwd_root) = self.resolve_abs_path(cwd_path)?;
        let resolved = drive.resolve_pathparts(relative, cwd_root)?;
        Ok((drive, resolved))
    }

    /// resolves path into a Drive and an Inode
    /// path may be relative to cwd or absolute
    #[must_use]
    #[inline]
    fn resolve_path(&self, path: Path) -> FSResult<(&Arc<dyn FileSystem>, Inode)> {
        if path.is_absolute() {
            self.resolve_abs_path(path)
        } else {
            let relative_parts = path.parts().unwrap_or_default();
            let state = this_state();
            let cwd = state.cwd();
            self.resolve_relative_path(cwd, relative_parts)
        }
    }

    #[must_use]
    #[inline]
    fn resolve_uncreated_path<'a, 'b>(
        &'a self,
        path: Path<'b>,
    ) -> FSResult<(&'a Arc<dyn FileSystem>, Inode, &'b str)> {
        let (name, path) = path.spilt_into_name();

        let name = name.ok_or(FSError::InvalidPath)?;
        let (drive, resolved) = self.resolve_path(path)?;
        if resolved.kind() != InodeType::Directory {
            return Err(FSError::NotADirectory);
        }

        Ok((drive, resolved, name))
    }

    /// checks if a path is a valid dir returns Err if path has an error
    pub fn verify_path_dir(&self, path: Path) -> FSResult<()> {
        let (_, res) = self.resolve_path(path)?;

        if !res.is_dir() {
            return Err(FSError::NotADirectory);
        }
        Ok(())
    }

    fn unpack_tar(&self, fs: &mut dyn FileSystem, tar: &mut TarArchiveIter) -> FSResult<()> {
        while let Some(inode) = tar.next() {
            let path = PathParts::new(inode.name());
            let root = fs.root_inode();

            if cfg!(debug_assertions) {
                debug!(VFS, "Unpacking ({}) {path} ...", inode.kind);
            }

            fn resolve_uncreated_path<'a>(
                path: PathParts<'a>,
                root: &Arc<dyn InodeOps>,
                fs: &mut dyn FileSystem,
            ) -> FSResult<(Inode, &'a str)> {
                let (name, parts) = path.spilt_into_name();

                let parent_node = fs.resolve_pathparts(parts, root.clone())?;
                Ok((parent_node, name.unwrap_or_default()))
            }

            match inode.kind {
                ustar::Type::NORMAL => {
                    let (parent_node, name) = resolve_uncreated_path(path, &root, fs)?;
                    fs.create(parent_node, name)?;

                    let node = fs.resolve_pathparts(path, root)?;
                    node.write(0, inode.data())?;
                    node.close();
                }

                ustar::Type::DIR => {
                    let (parent_node, name) = resolve_uncreated_path(path, &root, fs)?;
                    fs.createdir(parent_node, name)?
                }

                _ => return Err(FSError::OperationNotSupported),
            };
        }
        Ok(())
    }

    fn open(&self, path: Path) -> FSResult<FileDescriptor> {
        let (mountpoint, node) = self.resolve_path(path)?;

        mountpoint.on_open(path)?;
        node.opened();

        Ok(FileDescriptor::new(mountpoint.clone(), node))
    }

    fn createfile(&self, path: Path) -> FSResult<()> {
        let (mountpoint, node, name) = self.resolve_uncreated_path(path)?;
        mountpoint.create(node, name)
    }

    fn createdir(&self, path: Path) -> FSResult<()> {
        let (mountpoint, node, name) = self.resolve_uncreated_path(path)?;
        mountpoint.createdir(node, name)
    }

    pub fn mount_device(&self, path: Path, device: &'static dyn Device) -> FSResult<()> {
        let (mountpoint, node, name) = self.resolve_uncreated_path(path)?;
        mountpoint.mount_device(node, name, device)
    }

    pub fn get_direntry(&self, path: Path) -> FSResult<DirEntry> {
        let (_, node) = self.resolve_path(path)?;
        let parts = path.parts().unwrap_or_default();
        let (name, _) = parts.spilt_into_name();

        if let Some(name) = name {
            Ok(DirEntry::get_from_inode(node, name))
        } else {
            Ok(DirEntry::get_from_inode(node, ""))
        }
    }
}
