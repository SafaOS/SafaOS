//! Filesystem module
//!
//! This module provides a high-level interface for interacting with the virtual filesystem driver
//! (see [crate::drivers::vfs] and [crate::drivers::vfs::VFS_STRUCT])

use crate::{
    drivers::vfs::{FSResult, VFS_STRUCT},
    utils::path::Path,
};

mod file;
pub use file::*;

mod diriter;
pub use diriter::*;

use safa_abi::raw::io::DirEntry;

/// Get directory entry information for the specified path.
pub fn get_direntry(path: Path) -> FSResult<DirEntry> {
    VFS_STRUCT.read().get_direntry(path)
}

/// Creates a new directory at the specified path,
/// doesn't create parent directories if they don't exist.
pub fn createdir(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().createdir(path)
}

/// Creates a new file at the specified path,
/// doesn't create parent directories if they don't exist.
pub fn create(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().createfile(path)
}

/// Removes a file or directory at the specified path.
pub fn remove(path: Path) -> FSResult<()> {
    VFS_STRUCT.read().remove_path(path)
}
