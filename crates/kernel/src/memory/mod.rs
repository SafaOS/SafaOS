pub mod buddy_allocator;
pub mod frame_allocator;
pub mod page_allocator;
pub mod paging;
pub mod sorcery;

use core::{
    fmt::{Debug, LowerHex},
    ops::{Add, AddAssign, Deref, DerefMut, Sub, SubAssign},
    ptr::NonNull,
};

use paging::{EntryFlags, MapToError, Page, PageTable, PhysPageTable, PAGE_SIZE};
use safa_utils::abi::raw::RawSlice;
use serde::Serialize;

use crate::limine::HHDM;

// FIXME: Implementition of serialize should serialize as hex string because memory addresses don't fit in json's int
/// A virtual memory address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Default)]
#[repr(transparent)]
pub struct VirtAddr(usize);

/// A physical memory address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Default)]
#[repr(transparent)]
pub struct PhysAddr(usize);

impl Debug for VirtAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VirtAddr({self:#x})")
    }
}

impl Debug for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PhysAddr({self:#x})")
    }
}

macro_rules! impl_addr_ty {
    ($ty: ty) => {
        impl $ty {
            #[inline(always)]
            pub const fn null() -> Self {
                Self(0)
            }

            #[inline(always)]
            pub const fn from(value: usize) -> Self {
                Self(value)
            }

            #[inline(always)]
            pub const fn into_raw(self) -> usize {
                self.0
            }
        }

        impl LowerHex for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                LowerHex::fmt(&self.0, f)
            }
        }

        impl From<usize> for $ty {
            #[inline(always)]
            fn from(value: usize) -> Self {
                Self::from(value)
            }
        }

        impl Add<usize> for $ty {
            type Output = $ty;
            #[inline(always)]
            fn add(self, rhs: usize) -> Self::Output {
                Self(self.0 + rhs)
            }
        }

        impl Add<$ty> for $ty {
            type Output = $ty;
            #[inline(always)]
            fn add(self, rhs: $ty) -> Self::Output {
                self + rhs.0
            }
        }

        impl AddAssign<usize> for $ty {
            #[inline(always)]
            fn add_assign(&mut self, rhs: usize) {
                *self = *self + rhs
            }
        }

        impl Sub<$ty> for $ty {
            type Output = usize;
            #[inline(always)]
            fn sub(self, rhs: $ty) -> Self::Output {
                self.0 - rhs.0
            }
        }

        impl Sub<usize> for $ty {
            type Output = Self;
            #[inline(always)]
            fn sub(self, rhs: usize) -> Self::Output {
                Self(self.0 - rhs)
            }
        }

        impl SubAssign<usize> for $ty {
            #[inline(always)]
            fn sub_assign(&mut self, rhs: usize) {
                *self = *self - rhs
            }
        }

        impl Deref for $ty {
            type Target = usize;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

impl_addr_ty!(VirtAddr);
impl_addr_ty!(PhysAddr);

impl VirtAddr {
    #[inline(always)]
    pub fn from_ptr<T>(value: *const T) -> Self {
        Self(value as usize)
    }

    #[inline(always)]
    pub fn into_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    /// NOTE: it is unlikely that a VirtAddr would have an equalivent PhysAddr, it is safe to assume so if the VirtAddr was gathered [`PhysAddr::into_virt`]
    #[inline(always)]
    pub fn into_phys(self) -> PhysAddr {
        PhysAddr(self.0 - *HHDM)
    }

    #[inline(always)]
    pub fn from_phys(value: usize) -> VirtAddr {
        PhysAddr::from(value).into_virt()
    }
}

impl PhysAddr {
    #[inline(always)]
    pub fn into_virt(self) -> VirtAddr {
        VirtAddr(self.0 | *HHDM)
    }
}
impl<T> From<*const T> for VirtAddr {
    #[inline(always)]
    fn from(value: *const T) -> Self {
        Self::from_ptr(value)
    }
}

impl<T> From<*mut T> for VirtAddr {
    #[inline(always)]
    fn from(value: *mut T) -> Self {
        Self::from_ptr(value)
    }
}

#[inline(always)]
pub const fn align_up(address: usize, alignment: usize) -> usize {
    (address + alignment - 1) & !(alignment - 1)
}

#[inline(always)]
pub const fn align_down(x: usize, alignment: usize) -> usize {
    x & !(alignment - 1)
}

#[inline(always)]
pub fn copy_to_userspace(page_table: &mut PageTable, addr: VirtAddr, obj: &[u8]) {
    let pages_required = obj.len().div_ceil(PAGE_SIZE) + 1;
    let mut copied = 0;
    let mut to_copy = obj.len();

    for i in 0..pages_required {
        let page = Page::containing_address(addr + copied);
        let diff = if i == 0 { addr - page.virt_addr() } else { 0 };
        let will_copy = if (to_copy + diff) >= PAGE_SIZE {
            PAGE_SIZE - diff
        } else {
            to_copy
        };

        let frame = page_table.get_frame(page).unwrap();

        let virt_addr = frame.virt_addr() + diff;
        unsafe {
            core::ptr::copy_nonoverlapping(
                obj.as_ptr().byte_add(copied),
                virt_addr.into_ptr(),
                will_copy,
            );
        }

        copied += will_copy;
        to_copy -= will_copy;
    }
}

/// Maps the arguments (`slices`) to the environment area in the given page table.
/// returns an FFI safe pointer to the args array
/// returns None if arguments are empty
///
/// # Layout
/// directly at `start` is the argv length,
/// followed by the arg raw bytes ([u8]),
/// followed by the args pointers (RawSlice<u8>).
///
/// the returned slice is a slice of the argv pointers, meaning it is not available until the page table is loaded
/// there is an added null character at the end of each argument for compatibility with C
pub fn map_byte_slices(
    page_table: &mut PhysPageTable,
    slices: &[&[u8]],
    map_start_addr: VirtAddr,
) -> Result<Option<NonNull<RawSlice<u8>>>, MapToError> {
    if slices.is_empty() {
        return Ok(None);
    }

    let mut allocated_bytes_remaining = 0;
    let mut current_page = map_start_addr;

    let mut map_next = |page_table: &mut PhysPageTable, allocated_bytes_remaining: &mut usize| {
        let results = unsafe {
            page_table.map_to(
                Page::containing_address(current_page),
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )
        };
        *allocated_bytes_remaining += 4096;
        current_page += 4096;
        results
    };

    let mut map_next_bytes = |bytes: usize,
                              page_table: &mut PhysPageTable,
                              allocated_bytes_remaining: &mut usize|
     -> Result<(), MapToError> {
        let pages = (bytes + PAGE_SIZE - 1) / PAGE_SIZE;

        for _ in 0..pages {
            map_next(page_table, allocated_bytes_remaining)?;
        }
        Ok(())
    };

    macro_rules! map_if_not_enough {
        ($bytes: expr) => {
            if allocated_bytes_remaining < $bytes {
                map_next_bytes($bytes, page_table, &mut allocated_bytes_remaining)?;
            } else {
                allocated_bytes_remaining -= $bytes;
            }
        };
    }

    const USIZE_BYTES: usize = size_of::<usize>();
    map_if_not_enough!(8);
    let mut start_addr = map_start_addr;
    // argc
    copy_to_userspace(page_table, start_addr, &slices.len().to_ne_bytes());

    // argv*
    start_addr += USIZE_BYTES;

    for slice in slices {
        map_if_not_enough!(slice.len() + 1);

        copy_to_userspace(page_table, start_addr, slice);
        // null-terminate arg
        copy_to_userspace(page_table, start_addr + slice.len(), b"\0");
        start_addr += slice.len() + 1;
    }

    let mut start_addr: VirtAddr = start_addr.next_multiple_of(USIZE_BYTES).into();
    let slices_addr = start_addr;
    let mut current_slice_ptr = map_start_addr + USIZE_BYTES /* after argc */;

    for slice in slices {
        map_if_not_enough!(size_of::<RawSlice<u8>>());

        let raw_slice =
            unsafe { RawSlice::from_raw_parts(current_slice_ptr.into_ptr::<u8>(), slice.len()) };
        let bytes: [u8; size_of::<RawSlice<u8>>()] = unsafe { core::mem::transmute(raw_slice) };

        copy_to_userspace(page_table, start_addr, &bytes);
        start_addr += bytes.len();

        current_slice_ptr += slice.len() + 1; // skip the data (and null terminator)
    }

    Ok(Some(unsafe {
        NonNull::new_unchecked(slices_addr.into_ptr::<RawSlice<u8>>())
    }))
}

/// Same as [`map_byte_slices`] but for &str
pub fn map_str_slices(
    page_table: &mut PhysPageTable,
    args: &[&str],
    start_addr: VirtAddr,
) -> Result<Option<NonNull<RawSlice<u8>>>, MapToError> {
    return map_byte_slices(
        page_table,
        unsafe { core::mem::transmute(args) },
        start_addr,
    );
}
