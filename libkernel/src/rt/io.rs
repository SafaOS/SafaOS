use core::{mem::ManuallyDrop, ptr::NonNull};

use alloc::sync::Arc;

use crate::rt::abi::*;
use crate::rt::future::DynFut;
use macros::vtable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IOError {
    OffsetOutOfRange,
    OutOfMemory,
    NotImplemented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekWhence {
    Start(usize),
    End(usize),
    Current(usize),
}

type Result<T> = core::result::Result<T, IOError>;

fn ready<'a, T: Send + 'a>(v: T) -> DynFut<'a, T> {
    DynFut::new(core::future::ready(v))
}

fn not_implemented<'a, T: Send + 'a>() -> DynFut<'a, Result<T>> {
    ready(Err(IOError::NotImplemented))
}

#[vtable]
/// A handle to an open IO interface.
pub trait IOHandler {
    fn size<'a>(&'a self) -> DynFut<'a, Result<usize>> {
        not_implemented()
    }

    fn read<'a>(&'a self, _offset: usize, _buf: &'a mut [u8]) -> DynFut<'a, Result<usize>> {
        not_implemented()
    }
}

#[repr(C)]
/// An Opaque FFI safe IOHandle that refers to an [`IOHandler`].
pub struct IOHandle {
    private: OpaqueSelf<'static>,
    pub vtable: &'static IOHandlerVTable,
}

impl IOHandle {
    pub const fn this<'a>(&'a self) -> OpaqueSelf<'a> {
        self.private
    }
}

impl Drop for IOHandle {
    fn drop(&mut self) {
        unsafe { (self.vtable.drop)(self.this()) }
    }
}

impl Clone for IOHandle {
    fn clone(&self) -> Self {
        let new_ptr = unsafe { (self.vtable.clone)(self.this()) };
        Self {
            private: new_ptr,
            vtable: self.vtable,
        }
    }
}

impl IOHandler for IOHandle {
    fn size<'a>(&'a self) -> DynFut<'a, Result<usize>> {
        let f = self.vtable.size;
        unsafe { (f)(self.this()) }
    }

    fn read<'a>(&'a self, offset: usize, buf: &'a mut [u8]) -> DynFut<'a, Result<usize>> {
        let f = self.vtable.read;

        unsafe { (f)(self.this(), offset, CSliceMut::from(buf)) }
    }
}

const fn arc_vtable<T: IOHandler + 'static>() -> &'static IOHandlerVTable {
    const { &IOHandlerVTable::new_with::<T>(arc_drop::<T>, arc_clone::<T>) }
}

const fn zst_vtable<T: IOHandler + 'static>() -> &'static IOHandlerVTable {
    const { &IOHandlerVTable::new_with::<T>(noop_drop, zst_clone::<T>) }
}

unsafe extern "C" fn noop_drop(_ptr: OpaqueSelf) {}
unsafe extern "C" fn zst_clone<T>(_ptr: OpaqueSelf) -> OpaqueSelf<'static> {
    OpaqueSelf::new(NonNull::<T>::dangling().cast())
}

unsafe extern "C" fn arc_drop<T>(ptr: OpaqueSelf) {
    drop(unsafe { Arc::<T>::from_raw(ptr.as_ptr().as_ptr().cast()) });
}

unsafe extern "C" fn arc_clone<T>(ptr: OpaqueSelf) -> OpaqueSelf<'static> {
    unsafe {
        OpaqueSelf::new(
            NonNull::new_unchecked(
                Arc::into_raw(
                    (&*ManuallyDrop::new(Arc::<T>::from_raw(ptr.as_ptr().as_ptr().cast()))).clone(),
                )
                .cast_mut(),
            )
            .cast(),
        )
    }
}

impl IOHandle {
    #[inline]
    pub const fn from_zst<T: IOHandler + 'static>() -> Self {
        const {
            assert!(
                size_of::<T>() == 0,
                "from_zst can only be called on a zero-sized type"
            )
        };

        Self {
            private: OpaqueSelf::new(NonNull::dangling()),
            vtable: zst_vtable::<T>(),
        }
    }

    /// Returns a generic `IOHandle` over an Arc Handle.
    #[inline]
    pub fn from_arc<T: IOHandler + 'static>(arc: Arc<T>) -> Self {
        Self {
            private: unsafe {
                OpaqueSelf::new(NonNull::new_unchecked(Arc::into_raw(arc).cast_mut()).cast())
            },
            vtable: arc_vtable::<T>(),
        }
    }
}
