#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InodeType {
    File,
    Directory,
    Device,
}

// Keep in sync with kernel implementition in kernel::vfs::expose::FileAttr
// The ABI version cannot be used directely in the kernel implementition
#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FileAttr {
    pub kind: InodeType,
    pub size: usize,
}

// Keep in sync with kernel implementition in kernel::vfs::expose::DirEntry
// The ABI version cannot be used directely in the kernel implementition
#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct DirEntry {
    pub attrs: FileAttr,
    pub name_length: usize,
    pub name: [u8; super::consts::MAX_NAME_LENGTH],
}
