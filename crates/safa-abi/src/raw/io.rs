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
