//! This mod is a wrapper around the [`safa_utils`] crate
//! with a few additions

use core::ops::Deref;

pub use safa_utils::*;
use spin::lazy::Lazy;
pub mod alloc;
pub mod dtb;
pub mod elf;
pub mod locks;
pub mod ustar;

/// A wrapper around [`locks::Mutex`] which allows an outsider trait implementation
pub struct Locked<T: ?Sized> {
    inner: locks::Mutex<T>,
}

impl<T> Locked<T> {
    pub const fn new(inner: T) -> Self {
        Self {
            inner: locks::Mutex::new(inner),
        }
    }
}

impl<T> Deref for Locked<T> {
    type Target = locks::Mutex<T>;

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
    type Target = locks::Mutex<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
