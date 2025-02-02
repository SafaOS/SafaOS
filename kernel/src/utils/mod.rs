pub mod alloc;
pub mod ansi;
pub mod display;
pub mod either;
pub mod elf;
pub mod errors;
pub mod ffi;
pub mod io;
pub mod ustar;

use core::ops::Deref;

use serde::Serialize;
use spin::{Lazy, Mutex};

pub struct Locked<T: ?Sized> {
    inner: Mutex<T>,
}

impl<T> Locked<T> {
    pub const fn new(inner: T) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

impl<T> Deref for Locked<T> {
    type Target = Mutex<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// LazyLock is a wrapper around [`Lazy<Mutex<T>>`] that implements [`Deref`] to [`Mutex<T>`]
pub struct LazyLock<T> {
    inner: Lazy<Locked<T>>,
}

impl<T> LazyLock<T> {
    pub const fn new(inner: fn() -> Locked<T>) -> Self {
        Self {
            inner: Lazy::new(inner),
        }
    }
}

impl<T> Deref for LazyLock<T> {
    type Target = Mutex<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug, Clone)]
pub struct HeaplessString<const N: usize>(heapless::String<N>);
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
