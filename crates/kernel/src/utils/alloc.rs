extern crate alloc;

use core::fmt::Write;
use core::ops::{Deref, DerefMut};
use core::str;

use crate::memory::page_allocator::PageAlloc;
use alloc::vec::Vec;

use super::bstr::BStr;

#[derive(Debug, Clone)]
pub struct PageVec<T>(Vec<T, PageAlloc>);

impl<T> PageVec<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity_in(capacity, PageAlloc))
    }

    pub const fn new() -> Self {
        Self(Vec::new_in(PageAlloc))
    }
}

impl<T> Deref for PageVec<T> {
    type Target = Vec<T, PageAlloc>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for PageVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> AsMut<Vec<T, PageAlloc>> for PageVec<T> {
    fn as_mut(&mut self) -> &mut Vec<T, PageAlloc> {
        &mut self.0
    }
}

impl<T> AsRef<Vec<T, PageAlloc>> for PageVec<T> {
    fn as_ref(&self) -> &Vec<T, PageAlloc> {
        &self.0
    }
}

impl<T> From<Vec<T, PageAlloc>> for PageVec<T> {
    fn from(v: Vec<T, PageAlloc>) -> Self {
        Self(v)
    }
}

/// a non-utf8 string that uses the page allocator
pub struct PageBString {
    inner: PageVec<u8>,
}

impl PageBString {
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: PageVec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn push_str(&mut self, s: &str) {
        self.inner.extend_from_slice(s.as_bytes());
    }

    #[inline]
    pub fn push_char(&mut self, c: char) {
        let mut dst = [0; 4];
        let fake_str = c.encode_utf8(&mut dst);
        self.push_str(fake_str);
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_slice()
    }

    #[inline]
    pub fn push_bstr(&mut self, s: &BStr) {
        self.inner.extend_from_slice(s.as_bytes());
    }

    #[inline]
    pub fn push_bytes(&mut self, s: &[u8]) {
        self.inner.extend_from_slice(s);
    }

    #[inline]
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(self.as_bytes())
    }

    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

impl core::fmt::Write for PageBString {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push_str(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.push_char(c);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PageString {
    inner: PageVec<u8>,
}

impl PageString {
    pub fn new() -> Self {
        Self {
            inner: PageVec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: PageVec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn push_str(&mut self, s: &str) {
        self.inner.extend_from_slice(s.as_bytes());
    }

    pub fn push_char(&mut self, c: char) {
        let mut dst = [0; 4];
        let fake_str = c.encode_utf8(&mut dst);
        self.push_str(fake_str);
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.inner) }
    }
}

impl serde_json::io::Write for PageVec<u8> {
    fn write(&mut self, buf: &[u8]) -> serde_json::io::Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> serde_json::io::Result<()> {
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> serde_json::io::Result<()> {
        self.extend_from_slice(buf);
        Ok(())
    }
}

impl Write for PageString {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push_str(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.push_char(c);
        Ok(())
    }
}

impl serde_json::io::Write for PageString {
    fn write(&mut self, buf: &[u8]) -> serde_json::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> serde_json::io::Result<()> {
        self.inner.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> serde_json::io::Result<()> {
        self.inner.write_all(buf)
    }
}

impl Deref for PageString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        unsafe { core::str::from_utf8_unchecked(&self.inner) }
    }
}

impl DerefMut for PageString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { core::str::from_utf8_unchecked_mut(&mut self.inner) }
    }
}
