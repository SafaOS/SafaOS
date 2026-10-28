use core::{mem::ManuallyDrop, ptr::NonNull};

use alloc::sync::Arc;

use crate::ffi::*;
use crate::future::DynFut;
use macros::vtable;

/// Describes all interfaces that can be read from.
pub trait IORead {
    fn read_next(&mut self, buf: &mut [u8]) -> Result<usize, IOError>;
}

/// A trait for interfaces where you can change the read/write position.
pub trait IOSeek {
    fn seek(&mut self, wrench: SeekWhence) -> Result<u64, IOError>;
}

impl<T> IORead for core::io::Cursor<T>
where
    T: AsRef<[u8]>,
{
    fn read_next(&mut self, buf: &mut [u8]) -> Result<usize, IOError> {
        let position = self.position();

        let reader: &[u8] = &self.get_ref().as_ref()[position as usize..];
        let amount: usize = buf.len().min(reader.len());
        buf.copy_from_slice(&reader[..amount]);

        self.set_position(position + amount as u64);
        Ok(amount)
    }
}

impl<T> IOSeek for core::io::Cursor<T>
where
    T: AsRef<[u8]>,
{
    fn seek(&mut self, wrench: SeekWhence) -> Result<u64, IOError> {
        let len: usize = self.get_ref().as_ref().len();
        let position = self.position();
        let new_position = match wrench {
            SeekWhence::Current(am) => position + am,
            SeekWhence::End(e) => (len as u64).checked_sub(e).ok_or(IOError::OutOfBounds)?,
            SeekWhence::Start(s) => s,
        };

        if new_position > len as u64 {
            return Err(IOError::OutOfBounds);
        }

        self.set_position(new_position);
        Ok(new_position)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum IOError {
    OutOfBounds,
    OutOfMemory,
    NotImplemented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SeekWhence {
    Start(u64),
    End(u64),
    Current(u64),
}

type FResult<T> = FFIResult<T, IOError>;

fn ready<'a, T: Send + 'a>(v: T) -> DynFut<'a, T> {
    DynFut::new(core::future::ready(v))
}

fn not_implemented<'a, T: Send + 'a>() -> DynFut<'a, FResult<T>> {
    ready(FResult::Err(IOError::NotImplemented))
}

#[vtable]
/// A handle to an open IO interface.
pub trait IOHandler: Send + Sync {
    fn size<'a>(&'a self) -> DynFut<'a, FResult<usize>> {
        not_implemented()
    }

    fn read<'a>(&'a self, _offset: SeekWhence, _buf: &'a mut [u8]) -> DynFut<'a, FResult<usize>> {
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
    fn size<'a>(&'a self) -> DynFut<'a, FResult<usize>> {
        let f = self.vtable.size;
        unsafe { (f)(self.this()) }
    }

    fn read<'a>(&'a self, offset: SeekWhence, buf: &'a mut [u8]) -> DynFut<'a, FResult<usize>> {
        let f = self.vtable.read;

        unsafe { (f)(self.this(), offset, FFISliceMut::from(buf)) }
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

unsafe impl Send for IOHandle {}
unsafe impl Sync for IOHandle {}
