use core::fmt::{Debug, Display};

use crate::{
    debug,
    devices::{self, Device},
    error, limine,
    memory::{frame_allocator, paging::PAGE_SIZE},
    process, time,
    utils::{
        path::PathParts,
        ustar::{self, TarArchiveIter},
    },
};

use hashbrown::HashMap;
use safa_abi::{
    errors::{ErrorStatus, IntoErr},
    fs::{DirEntry, FileAttr},
};
use thiserror::Error;

pub mod ramfs;
pub mod rodatafs;
// TODO: write more tests
#[cfg(test)]
pub mod tests;

use crate::utils::locks::RwLock;
use crate::utils::path::Path;
use crate::utils::{
    path::PathError,
    types::{DriveName, FileName},
};
use alloc::{boxed::Box, sync::Arc};
use lazy_static::lazy_static;
use safa_abi::fs::OpenOptions;

lazy_static! {
    pub static ref VFS_STRUCT: RwLock<VFS> = RwLock::new(VFS::create());
}

#[derive(Debug, Clone, Copy)]
pub enum SeekOffset {
    Start(usize),
    End(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FSError {
    NotFound,
    NotAFile,
    NotADirectory,
    DirectoryNotEmpty,
    InvalidOffset,
    InvalidPath,
    InvalidName,
    /// When the File System Label is not found
    FSLabelNotFound,
    PathTooLong,
    MissingPermission,
    OperationNotSupported,
    InvalidResource,
    AlreadyExists,
    InvalidCmd,
    InvalidArg,
    NotExecutable,
}

impl Display for FSError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl IntoErr for FSError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::MissingPermission => ErrorStatus::MissingPermissions,
            Self::DirectoryNotEmpty => ErrorStatus::DirectoryNotEmpty,
            Self::OperationNotSupported => ErrorStatus::OperationNotSupported,
            Self::NotAFile => ErrorStatus::NotAFile,
            Self::NotADirectory => ErrorStatus::NotADirectory,
            Self::NotFound => ErrorStatus::NoSuchAFileOrDirectory,
            Self::InvalidPath => ErrorStatus::InvalidPath,
            Self::FSLabelNotFound => ErrorStatus::NoSuchAFileOrDirectory,
            Self::AlreadyExists => ErrorStatus::AlreadyExists,
            Self::NotExecutable => ErrorStatus::NotExecutable,
            Self::InvalidOffset => ErrorStatus::InvalidOffset,
            Self::InvalidCmd => ErrorStatus::InvalidCommand,
            Self::InvalidArg => ErrorStatus::InvalidArgument,
            Self::InvalidName | Self::PathTooLong => ErrorStatus::StrTooLong,
            Self::InvalidResource => ErrorStatus::InvalidResource,
        }
    }
}

impl From<PathError> for FSError {
    fn from(value: PathError) -> Self {
        match value {
            PathError::DriveNameTooLong => Self::FSLabelNotFound,
            PathError::PathPartsTooLong => Self::PathTooLong,
            PathError::FailedToJoinPaths | PathError::InvalidPath => Self::InvalidPath,
        }
    }
}

pub type FSResult<T> = Result<T, FSError>;

/// Represents the unique identifier for a file system object, that is a unique identifier for each file returned by the path resolution.
pub type FSObjectID = usize;

#[derive(Debug, Clone)]
/// A descriptor for a currently open file system object, that is a combination of a [`VFSObjectID`] and the open options.
pub struct FSObjectDescriptor {
    id: VFSObjectID,
    options: OpenOptions,
}

impl FSObjectDescriptor {
    pub fn write(&self, offset: SeekOffset, data: &[u8]) -> FSResult<usize> {
        if !self.options.is_write() {
            return Err(FSError::MissingPermission);
        }

        self.id.write(offset, data)
    }

    pub fn truncate(&self, size: usize) -> FSResult<()> {
        if !self.options.is_write() {
            return Err(FSError::MissingPermission);
        }

        self.id.truncate(size)
    }

    pub fn read(&self, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        if !self.options.is_read() {
            return Err(FSError::MissingPermission);
        }

        self.id.read(offset, buf)
    }

    pub fn open_collection_iter(&self) -> FSResult<CollectionIterDescriptor> {
        self.id.open_collection_iter()
    }

    pub fn attrs(&self) -> FileAttr {
        self.id.attrs()
    }

    pub fn kind(&self) -> FSObjectType {
        self.attrs().kind
    }

    pub fn size(&self) -> usize {
        self.attrs().size
    }

    pub fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        self.id.send_command(cmd, arg)
    }

    pub fn sync(&self) -> FSResult<()> {
        self.id.sync()
    }
}

#[derive(Debug, Clone)]
/// Represents a unique identifier for a VFS object, that is a combination of [`FSObjectID`] and the file system itself.
struct VFSObjectID {
    fs_obj_id: FSObjectID,
    fs: Arc<dyn FileSystem>,
}

impl VFSObjectID {
    pub fn write(&self, offset: SeekOffset, data: &[u8]) -> FSResult<usize> {
        self.fs.write(self.fs_obj_id, offset, data)
    }

    pub fn read(&self, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        self.fs.read(self.fs_obj_id, offset, buf)
    }

    pub fn truncate(&self, size: usize) -> FSResult<()> {
        self.fs.truncate(self.fs_obj_id, size)
    }

    pub fn open_collection_iter(&self) -> FSResult<CollectionIterDescriptor> {
        let children_collection = self.fs.get_children(self.fs_obj_id)?;
        Ok(CollectionIterDescriptor::new(children_collection))
    }

    pub fn attrs(&self) -> FileAttr {
        self.fs.attrs_of(self.fs_obj_id)
    }

    pub fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        self.fs.send_command(self.fs_obj_id, cmd, arg)
    }

    pub fn sync(&self) -> FSResult<()> {
        self.fs.sync(self.fs_obj_id)
    }
}

impl Drop for VFSObjectID {
    fn drop(&mut self) {
        _ = self.sync();

        self.fs.on_close(self.fs_obj_id).unwrap_or_else(|e| {
            error!(
                VFS,
                "failed to close a file which should never happen: {:?}", e
            )
        });
    }
}

/// A descriptor of the items of an open FS Object Collection (Directory)
#[derive(Debug, Clone)]
pub struct CollectionIterDescriptor {
    entries: Box<[DirEntry]>,
    index: usize,
}

impl CollectionIterDescriptor {
    const fn new(entries: Box<[DirEntry]>) -> Self {
        Self { entries, index: 0 }
    }

    pub fn next(&mut self) -> Option<DirEntry> {
        let index = self.index;
        self.index += 1;
        self.entries.get(index).cloned()
    }
}

pub use safa_abi::fs::FSObjectType;

pub fn resolve_path_parts<F>(
    root_obj_id: FSObjectID,
    path: PathParts,
    get_dir_object: F,
) -> FSResult<FSObjectID>
where
    F: Fn(FSObjectID, FSObjectID, &str) -> FSResult<FSObjectID>,
{
    if path.is_empty() {
        return Ok(root_obj_id);
    }

    let mut current_object_id = root_obj_id;
    let mut grandparent_parent_id = root_obj_id;

    for depth in path.iter() {
        // quick optimization
        // remember these aren't removed by the path simplification always so you need sym/hardlinks to resolve them
        // FIXME: maybe remove the '.' optimization
        if depth == "." || depth == "" {
            continue;
        }
        let new_object_id = get_dir_object(grandparent_parent_id, current_object_id, depth)?;
        grandparent_parent_id = current_object_id;
        current_object_id = new_object_id;
    }

    Ok(current_object_id)
}

pub trait FileSystem: Send + Sync {
    fn name(&self) -> &'static str {
        "UnknownFS"
    }

    fn write(&self, id: FSObjectID, offset: SeekOffset, data: &[u8]) -> FSResult<usize> {
        _ = id;
        _ = offset;
        _ = data;
        Err(FSError::OperationNotSupported)
    }

    fn read(&self, id: FSObjectID, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        _ = id;
        _ = offset;
        _ = buf;
        Err(FSError::OperationNotSupported)
    }

    fn truncate(&self, id: FSObjectID, size: usize) -> FSResult<()> {
        _ = id;
        _ = size;
        Err(FSError::OperationNotSupported)
    }

    fn create_file(&self, parent_id: FSObjectID, name: &str) -> FSResult<FSObjectID> {
        _ = parent_id;
        _ = name;
        Err(FSError::OperationNotSupported)
    }

    fn create_directory(&self, parent_id: FSObjectID, name: &str) -> FSResult<FSObjectID> {
        _ = parent_id;
        _ = name;
        Err(FSError::OperationNotSupported)
    }

    fn mount_device(
        &self,
        parent_id: FSObjectID,
        name: &str,
        device: &'static dyn Device,
    ) -> FSResult<FSObjectID> {
        _ = parent_id;
        _ = name;
        _ = device;
        Err(FSError::OperationNotSupported)
    }

    fn remove(&self, child_name: &str, parent_id: FSObjectID, id: FSObjectID) -> FSResult<()> {
        _ = parent_id;
        _ = child_name;
        _ = id;
        Err(FSError::OperationNotSupported)
    }

    fn get_children(&self, id: FSObjectID) -> FSResult<Box<[DirEntry]>> {
        _ = id;
        Err(FSError::OperationNotSupported)
    }

    fn send_command(&self, id: FSObjectID, cmd: u16, arg: u64) -> FSResult<()> {
        _ = id;
        _ = cmd;
        _ = arg;

        Err(FSError::OperationNotSupported)
    }

    /// returns the [`FileAttr`] of a file system object
    fn attrs_of(&self, id: FSObjectID) -> FileAttr;

    /// returns the [`DirEntry`] of a file system object under a given name
    fn direntry_of(&self, id: FSObjectID, under_name: &str) -> FSResult<DirEntry> {
        let attrs = self.attrs_of(id);
        Ok(DirEntry::new(under_name, attrs))
    }

    fn resolve_path_rel(&self, parent_id: FSObjectID, path: PathParts) -> FSResult<FSObjectID>;

    fn on_open(&self, id: FSObjectID) -> FSResult<()>;

    fn on_close(&self, id: FSObjectID) -> FSResult<()>;

    fn sync(&self, id: FSObjectID) -> FSResult<()> {
        _ = id;
        Err(FSError::OperationNotSupported)
    }

    #[inline(always)]
    fn root_object_id(&self) -> FSObjectID {
        0
    }
}

impl dyn FileSystem + '_ {
    fn create_file_path(&self, path: PathParts) -> FSResult<FSObjectID> {
        let (name, parent_path) = path.spilt_into_name();
        let Some(name) = name else {
            return Err(FSError::InvalidPath);
        };

        let parent_id = self.resolve_path_rel(self.root_object_id(), parent_path)?;
        self.create_file(parent_id, name)
    }

    fn create_directory_path(&self, path: PathParts) -> FSResult<FSObjectID> {
        let (name, parent_path) = path.spilt_into_name();
        let Some(name) = name else {
            return Err(FSError::InvalidPath);
        };

        let parent_id = self.resolve_path_rel(self.root_object_id(), parent_path)?;
        self.create_directory(parent_id, name)
    }

    fn resolve_path_abs(&self, path: PathParts) -> FSResult<FSObjectID> {
        self.resolve_path_rel(self.root_object_id(), path)
    }
}

impl Debug for dyn FileSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

#[allow(clippy::upper_case_acronyms)]
pub struct VFS {
    mountpoints: HashMap<DriveName, Arc<dyn FileSystem>>,
}

impl VFS {
    pub fn new() -> Self {
        Self {
            mountpoints: HashMap::new(),
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
        let tempfs = RwLock::new(ramfs::RamFS::create());
        this.mount(DriveName::new_const("tmp"), tempfs).unwrap();
        // ramfs
        let ramfs = RwLock::new(ramfs::RamFS::create());
        this.mount(DriveName::new_const("ram"), ramfs).unwrap();

        devices::init(&mut this);
        // processes
        this.mount(
            DriveName::new_const("rod"),
            RwLock::new(rodatafs::RodFS::create()),
        )
        .unwrap();
        // ramdisk
        let mut ramdisk = limine::get_ramdisk();
        let mut ramfs = RwLock::new(ramfs::RamFS::create());

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
        self.mountpoints
            .try_insert(name, Arc::new(value))
            .map_err(|_| ())?;
        Ok(())
    }

    /// gets mountpoint from a path
    /// path must be absolute, or else it'd panic
    #[must_use]
    #[inline]
    fn get_mountpoint_from_path(&self, path: Path) -> FSResult<&Arc<dyn FileSystem>> {
        let drive = path.drive().expect("path is not absolute");
        let drive = self
            .mountpoints
            .get(drive)
            .ok_or(FSError::FSLabelNotFound)?;
        Ok(drive)
    }

    #[must_use]
    #[inline]
    fn resolve_abs_path(&self, path: Path) -> FSResult<(&Arc<dyn FileSystem>, FSObjectID)> {
        let mountpoint = self.get_mountpoint_from_path(path)?;
        let Some(parts) = path.parts() else {
            // empty paths always point to drive_root, little optimization
            return Ok((mountpoint, mountpoint.root_object_id()));
        };

        let resolved_id = mountpoint.resolve_path_abs(parts)?;

        Ok((mountpoint, resolved_id))
    }

    #[must_use]
    #[inline]
    /// Tries to resolve pathparts relative to `cwd_path` to a mountpoint and a FS Object ID
    fn resolve_relative_path(
        &self,
        cwd_path: Path,
        relative: PathParts,
    ) -> FSResult<(&Arc<dyn FileSystem>, FSObjectID)> {
        let (mountpoint, cwd_root) = self.resolve_abs_path(cwd_path)?;
        let resolved_id = mountpoint.resolve_path_rel(cwd_root, relative)?;
        Ok((mountpoint, resolved_id))
    }

    /// resolves path into a mountpoint and a FS Object ID
    /// path may be relative to cwd or absolute
    #[must_use]
    #[inline]
    fn resolve_path(&self, path: Path) -> FSResult<(&Arc<dyn FileSystem>, FSObjectID)> {
        if path.is_absolute() {
            self.resolve_abs_path(path)
        } else {
            let relative_parts = path.parts().unwrap_or_default();

            let process = process::current();
            let cwd = process.cwd();
            self.resolve_relative_path(cwd.as_path(), relative_parts)
        }
    }

    /// Resolves a path that may not exist into a parent mountpoint, a parent Object ID, and a name
    #[must_use]
    #[inline]
    fn resolve_uncreated_path<'a, 'b>(
        &'a self,
        path: Path<'b>,
    ) -> FSResult<(&'a Arc<dyn FileSystem>, FSObjectID, &'b str)> {
        let (name, path) = path.spilt_into_name();

        let name = name.ok_or(FSError::InvalidPath)?;
        let (mountpoint, resolved_id) = self.resolve_path(path)?;

        Ok((mountpoint, resolved_id, name))
    }

    /// checks if a path is a valid dir returns Err if path has an error
    pub fn verify_path_dir(&self, path: Path) -> FSResult<()> {
        let (mountpoint, res) = self.resolve_path(path)?;

        let kind = mountpoint.attrs_of(res).kind;
        if kind != FSObjectType::Directory {
            return Err(FSError::NotADirectory);
        }
        Ok(())
    }

    fn unpack_tar(&self, fs: &mut dyn FileSystem, tar: &mut TarArchiveIter) -> FSResult<()> {
        while let Some(inode) = tar.next() {
            let path = PathParts::new(inode.name());

            if cfg!(debug_assertions) {
                debug!(
                    VFS,
                    "Unpacking ({}) {path} ({}KiB) ...",
                    inode.kind,
                    inode.data().len() / 1024
                );
            }

            match inode.kind {
                ustar::Type::NORMAL => {
                    let id = fs.create_file_path(path)?;
                    fs.write(id, SeekOffset::Start(0), inode.data())?;
                }
                ustar::Type::DIR => {
                    let _ = fs.create_directory_path(path)?;
                }
                _ => return Err(FSError::OperationNotSupported),
            };
        }
        Ok(())
    }

    fn open_raw(&self, path: Path, create_file: bool, create_dir: bool) -> FSResult<VFSObjectID> {
        let (mountpoint, obj_id) = match self.resolve_path(path) {
            Ok((mountpoint, obj_id)) => (mountpoint, obj_id),
            Err(FSError::NotFound) if create_file => {
                let (mountpoint, parent_obj_id, name) = self.resolve_uncreated_path(path)?;
                let obj_id = mountpoint.create_file(parent_obj_id, name)?;
                (mountpoint, obj_id)
            }
            Err(FSError::NotFound) if create_dir => {
                let (mountpoint, parent_obj_id, name) = self.resolve_uncreated_path(path)?;
                let obj_id = mountpoint.create_directory(parent_obj_id, name)?;
                (mountpoint, obj_id)
            }
            Err(err) => return Err(err),
        };

        mountpoint.on_open(obj_id)?;
        Ok(VFSObjectID {
            fs_obj_id: obj_id,
            fs: mountpoint.clone(),
        })
    }

    pub fn open_all(&self, path: Path) -> FSResult<FSObjectDescriptor> {
        self.open(path, OpenOptions::READ | OpenOptions::WRITE)
    }

    pub fn open(&self, path: Path, options: OpenOptions) -> FSResult<FSObjectDescriptor> {
        let raw_id = self.open_raw(path, options.create_file(), options.create_dir())?;
        let descriptor = FSObjectDescriptor {
            id: raw_id,
            options,
        };

        if options.is_write_truncate() {
            _ = descriptor.truncate(0);
        }

        Ok(descriptor)
    }

    pub fn remove_path(&self, path: Path) -> FSResult<()> {
        let (mountpoint, parent_obj_id, name) = self.resolve_uncreated_path(path)?;
        let obj_id = mountpoint.resolve_path_rel(parent_obj_id, PathParts::new(name))?;
        mountpoint.remove(name, parent_obj_id, obj_id)
    }

    pub fn createfile(&self, path: Path) -> FSResult<()> {
        let (mountpoint, parent_obj_id, name) = self.resolve_uncreated_path(path)?;
        mountpoint.create_file(parent_obj_id, name).map(|_| ())
    }

    pub fn createdir(&self, path: Path) -> FSResult<()> {
        let (mountpoint, parent_obj_id, name) = self.resolve_uncreated_path(path)?;
        mountpoint.create_directory(parent_obj_id, name).map(|_| ())
    }

    pub fn mount_device(&self, path: Path, device: &'static dyn Device) -> FSResult<()> {
        let (mountpoint, parent_obj_id, name) = self.resolve_uncreated_path(path)?;
        mountpoint
            .mount_device(parent_obj_id, name, device)
            .map(|_| ())
    }

    pub fn get_direntry(&self, path: Path) -> FSResult<DirEntry> {
        let parts = path.parts().unwrap_or_default();
        let (mountpoint, obj_id) = self.resolve_path(path)?;
        let (name, _) = parts.spilt_into_name();

        let name = name.unwrap_or_default();
        mountpoint.direntry_of(obj_id, name)
    }
}
