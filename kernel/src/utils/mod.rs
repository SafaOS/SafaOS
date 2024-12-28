pub mod alloc;
pub mod ansi;
pub mod display;
pub mod either;
pub mod elf;
pub mod errors;
pub mod expose;
pub mod ffi;
pub mod ustar;

use core::ops::Deref;

use spin::Mutex;

pub struct Locked<T: ?Sized> {
    pub inner: Mutex<T>,
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
