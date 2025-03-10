#![no_std]
pub mod ansi;
pub mod bstr;
pub mod display;
pub mod either;
pub mod errors;
pub mod io;
pub mod path;
pub mod syscalls;
pub mod ustar;

use core::{
    borrow::Borrow,
    fmt::{Debug, Display, Write},
    ops::Deref,
    str::FromStr,
};

use serde::Serialize;

#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeaplessString<const N: usize>(heapless::String<N>);
impl<const N: usize> HeaplessString<N> {
    pub const fn new() -> Self {
        Self(heapless::String::new())
    }
}

impl<const N: usize> Write for HeaplessString<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_str(s)
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.0.write_char(c)
    }
}

impl<const N: usize> FromStr for HeaplessString<N> {
    type Err = <heapless::String<N> as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(heapless::String::from_str(s)?))
    }
}

impl<const N: usize> From<heapless::String<N>> for HeaplessString<N> {
    fn from(s: heapless::String<N>) -> Self {
        Self(s)
    }
}

impl<const N: usize> Deref for HeaplessString<N> {
    type Target = heapless::String<N>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> AsRef<str> for HeaplessString<N> {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl<const N: usize> AsMut<str> for HeaplessString<N> {
    fn as_mut(&mut self) -> &mut str {
        self.0.as_mut_str()
    }
}

impl<const N: usize> Serialize for HeaplessString<N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.as_str().trim_matches('\0'))
    }
}

impl<const N: usize> Display for HeaplessString<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<const N: usize> Debug for HeaplessString<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<const N: usize> Borrow<str> for HeaplessString<N> {
    fn borrow(&self) -> &str {
        &self.0
    }
}
