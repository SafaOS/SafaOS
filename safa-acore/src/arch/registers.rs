use crate::percpu::CpuLocal;

use super::inner;

#[inline(always)]
/// Returns a reference to the [`CpuLocal`] for this CPU.
pub fn cpu_local() -> &'static CpuLocal {
    inner::registers::cpu_local()
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StackFrame<'a> {
    inner: &'a inner::registers::StackFrame,
}

impl<'a> StackFrame<'a> {
    /// Converts a given pointer to a stack frame reference.
    #[inline(always)]
    pub unsafe fn from_fp(ptr: *const u8) -> Option<Self> {
        unsafe { inner::registers::StackFrame::from_fp(ptr).map(|p| Self { inner: p }) }
    }
    /// Gets the current Frame Pointer from the fp register
    #[inline(always)]
    pub unsafe fn get_current() -> Self {
        unsafe {
            Self {
                inner: inner::registers::StackFrame::get_current(),
            }
        }
    }

    #[inline(always)]
    /// Like [`Self::return_ptr`] but returns a pointer to the calling instruction instead (or an aproximation)
    pub fn call_ptr(&self) -> *mut u8 {
        self.inner.call_ptr()
    }

    /// Gets the previous stack frame from this one
    #[inline(always)]
    pub unsafe fn prev(&self) -> Option<Self> {
        unsafe { self.inner.prev().map(|f| Self { inner: f }) }
    }
}
