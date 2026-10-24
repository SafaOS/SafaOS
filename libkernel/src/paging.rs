use core::fmt::Debug;

use bitflags::bitflags;
use thiserror::Error;

use crate::pmm::PMMError;

/// Address Space identifier.
pub type ASID = u16;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MapToError {
    #[error("frame allocator: out of memory")]
    OutOfMemory,
    #[error("fatal: attempt to map an already mapped region")]
    AlreadyMapped,
    #[error("fatal: attempt to unmap an unmapped region")]
    NotMapped,
}

impl From<PMMError> for MapToError {
    fn from(value: PMMError) -> Self {
        match value {
            PMMError::OutOfMemory => Self::OutOfMemory,
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct PageEntryFlags: u32 {
        /// Page is read/write instead of read-only.
        const WRITE = 1;
        /// Userspace (Ring 0, EL0, etc) has access to this page.
        const USER_ACCESSIBLE = 1 << 1;
        /// No one is allowed to execute code from this page.
        const DISABLE_EXEC = 1 << 2;
        /// Usually indicates that caching is disabled.
        const DEVICE_UNCACHEABLE = 1 << 3;
        /// Usually indicates write-combining accelerated caching.
        const FRAMEBUFFER_CACHED = 1 << 4;
        /// This should probably be never passed manually, it indicates that when unmapping/changing page flags, some entries may still be invalid, and would be filled lazily.
        const _IS_LAZY = 1 << 5;
        /// This is a hint flag that the page we are mapping is shared globally and can be ignored by implementations.
        ///
        /// The use case is with x86_64 for example where the higher half is shared between different processes and you want the TLB to work more efficentlly with it,
        ///
        /// other architectures such as aarch64 may provide other solutions such as 2 tables for each halves which we support.
        const _GLOBAL_SHARED = 1 << 6;
    }
}
