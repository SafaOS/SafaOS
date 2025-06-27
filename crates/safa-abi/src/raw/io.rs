use core::ops::BitOr;

use crate::consts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FSObjectType {
    File,
    Directory,
    Device,
}

// Keep in sync with kernel implementition in kernel::vfs::expose::FileAttr
// The ABI version cannot be used directly in the kernel implementition
#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FileAttr {
    pub kind: FSObjectType,
    pub size: usize,
}

impl FileAttr {
    pub const fn new(kind: FSObjectType, size: usize) -> Self {
        Self { kind, size }
    }
}

// Keep in sync with kernel implementition in kernel::vfs::expose::DirEntry
// The ABI version cannot be used directly in the kernel implementition
#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct DirEntry {
    pub attrs: FileAttr,
    pub name_length: usize,
    pub name: [u8; consts::MAX_NAME_LENGTH],
}

impl DirEntry {
    pub fn new(name: &str, attrs: FileAttr) -> Self {
        let name_length = name.len().min(consts::MAX_NAME_LENGTH);
        let mut name_bytes = [0u8; consts::MAX_NAME_LENGTH];
        name_bytes[..name_length].copy_from_slice(name.as_bytes());
        Self {
            attrs,
            name_length,
            name: name_bytes,
        }
    }
}

/// Describes the options for opening a file or directory.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(transparent)]
pub struct OpenOptions(u8);

impl BitOr for OpenOptions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl OpenOptions {
    /// Open the file for writing.
    pub const WRITE: Self = Self(1 << 0);
    /// Open the file for reading.
    pub const READ: Self = Self(1 << 1);
    /// Create the file if it does not exist.
    pub const CREATE_FILE: Self = Self(1 << 2);
    /// Create the directory if it does not exist. (doesn't create parent directories)
    pub const CREATE_DIRECTORY: Self = Self(1 << 3);
    /// Truncate the file to zero length if it already exists.
    pub const WRITE_TRUNCATE: Self = Self(1 << 4);
    // no append because the user would provide the offset anyways

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_write(&self) -> bool {
        self.contains(Self::WRITE)
    }

    pub const fn is_write_truncate(&self) -> bool {
        self.contains(Self::WRITE_TRUNCATE)
    }

    pub const fn is_read(&self) -> bool {
        self.contains(Self::READ)
    }

    pub const fn create_file(&self) -> bool {
        self.contains(Self::CREATE_FILE)
    }

    pub const fn create_dir(&self) -> bool {
        self.contains(Self::CREATE_DIRECTORY)
    }
}
