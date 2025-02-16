use core::{
    fmt::{Display, Write},
    ops::{Deref, Index},
    str::Utf8Chunks,
};
#[repr(transparent)]
/// a non-utf8 byte str
pub struct BStr {
    inner: [u8],
}

impl BStr {
    #[inline(always)]
    pub fn new<B>(b: &B) -> &BStr
    where
        B: AsRef<[u8]> + ?Sized,
    {
        Self::from_bytes(b.as_ref())
    }
    #[inline(always)]
    pub fn from_bytes(b: &[u8]) -> &BStr {
        unsafe { &*(b as *const [u8] as *const BStr) }
    }
    #[inline]
    // TODO: implement Chars method for faster utf8 iteration
    pub fn utf8_chunks(&self) -> Utf8Chunks {
        self.inner.utf8_chunks()
    }
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }
}

impl Deref for BStr {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

impl Index<usize> for BStr {
    type Output = u8;
    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        self.inner.index(index)
    }
}

impl Index<core::ops::Range<usize>> for BStr {
    type Output = BStr;
    #[inline]
    fn index(&self, index: core::ops::Range<usize>) -> &Self::Output {
        BStr::from_bytes(&self.inner[index])
    }
}

impl Index<core::ops::RangeTo<usize>> for BStr {
    type Output = BStr;
    #[inline]
    fn index(&self, index: core::ops::RangeTo<usize>) -> &Self::Output {
        BStr::from_bytes(&self.inner[index])
    }
}

impl Index<core::ops::RangeFrom<usize>> for BStr {
    type Output = BStr;
    #[inline]
    fn index(&self, index: core::ops::RangeFrom<usize>) -> &Self::Output {
        BStr::from_bytes(&self.inner[index])
    }
}

impl<'a, T: AsRef<[u8]>> From<&'a T> for &'a BStr {
    #[inline]
    fn from(t: &'a T) -> Self {
        BStr::new(t)
    }
}

impl<'a> From<&'a str> for &'a BStr {
    #[inline]
    fn from(t: &'a str) -> Self {
        BStr::new(t)
    }
}

impl Display for BStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for chunk in self.utf8_chunks() {
            let valid = chunk.valid();
            let invaild = chunk.invalid();
            f.write_str(valid)?;
            if !invaild.is_empty() {
                f.write_char('\u{FFFD}')?;
            }
        }
        Ok(())
    }
}
