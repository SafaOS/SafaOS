//! This mod is a wrapper around the [`safa_utils`] crate
//! with a few additions

use core::ops::Deref;

pub use safa_utils::*;
pub mod alloc;
pub mod elf;
pub mod ustar;

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
