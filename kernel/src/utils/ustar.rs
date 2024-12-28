use core::{fmt::Debug, str};

use macros::display_consts;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Type(u8);

#[display_consts]
impl Type {
    pub const NORMAL: Self = Self(b'0');
    pub const HARD_LINK: Self = Self(b'1');
    pub const SOFT_LINK: Self = Self(b'2');
    pub const CHAR_DEV: Self = Self(b'3');
    pub const BLOCK_DEV: Self = Self(b'4');
    pub const DIR: Self = Self(b'5');
    pub const PIPE: Self = Self(b'6');
}

#[repr(C, packed)]
pub struct Inode {
    name: [u8; 100],

    mode: u64,
    owner_id: u64,
    user_id: u64,
    /// octal size in ascii
    /// what?
    size: [u8; 12],
    last_modified: [u8; 12],

    checksum: u64,
    pub kind: Type,
    linked_name: [u8; 100],

    ustar_magic: [u8; 6],
    ustar_version: [u8; 2],

    owner_name: [u8; 32],
    group_name: [u8; 32],

    device_major_number: u64,
    device_minor_number: u64,
    name_prefix: [u8; 155],
}

impl Debug for Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {} sized {}", self.name(), self.kind, self.size())
    }
}
impl Inode {
    #[inline]
    pub fn name(&self) -> &str {
        unsafe { str::from_utf8_unchecked(&self.name).trim_end_matches('\0') }
    }

    #[inline]
    pub fn size(&self) -> usize {
        let str = unsafe { str::from_utf8_unchecked(&self.size) }.trim_end_matches('\0');

        u32::from_str_radix(str, 8).unwrap() as usize
    }

    #[inline]
    pub fn verify(&self) -> bool {
        self.ustar_magic[0..5] == *b"ustar"
    }

    #[inline]
    unsafe fn data_ptr(this: *const Self) -> *const u8 {
        this.byte_add(512) as *const u8
    }

    #[inline]
    pub fn data(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(Self::data_ptr(self), self.size()) }
    }

    #[inline]
    pub unsafe fn next(this: *const Self) -> *const Inode {
        let filesize = (*this).size();

        let at = (filesize + 511) / 512;
        let next = (at + 1) * 512;

        this.byte_add(next)
    }
}

#[derive(Debug)]
pub struct TarArchiveIter<'a> {
    at: Option<&'a Inode>,
}

impl TarArchiveIter<'_> {
    /// safe warpper around Inode::next that verifies the inode before returning it
    /// also returns a refrence instead of a pointer
    pub fn next(&mut self) -> Option<&Inode> {
        let ret = self.at?;
        let next_ptr = unsafe { Inode::next(ret) };
        let next = unsafe { &*next_ptr };

        self.at = if next.verify() { Some(next) } else { None };
        Some(ret)
    }

    /// makes a new tar archive from ptr
    /// unsafe because ptr has to be mapped and non-null
    pub unsafe fn new(ptr: *const u8) -> Self {
        let at = &*(ptr as *const Inode);
        assert!(at.verify());

        Self { at: Some(at) }
    }
}
