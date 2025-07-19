pub mod buddy_allocator;
pub mod frame_allocator;
pub mod page_allocator;
pub mod paging;
pub mod proc_mem_allocator;
pub mod sorcery;

use core::{
    fmt::{Debug, LowerHex},
    ops::{Add, AddAssign, Deref, DerefMut, Sub, SubAssign},
};

use paging::{PAGE_SIZE, Page, PageTable};
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
            pub const fn into_bits(self) -> usize {
                self.0
            }

            #[inline(always)]
            pub const fn into_raw(self) -> usize {
                self.0
            }

            #[inline(always)]
            pub const fn from_bits(bits: usize) -> Self {
                Self(bits)
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

        impl const Add<usize> for $ty {
            type Output = $ty;
            #[inline(always)]
            fn add(self, rhs: usize) -> Self::Output {
                Self(self.0 + rhs)
            }
        }

        impl const Add<$ty> for $ty {
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

        impl const Sub<$ty> for $ty {
            type Output = usize;
            #[inline(always)]
            fn sub(self, rhs: $ty) -> Self::Output {
                self.0 - rhs.0
            }
        }

        impl const Sub<usize> for $ty {
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

        impl const AlignTo<usize> for $ty {
            #[inline(always)]
            fn to_next_multiple_of(self, alignment: usize) -> Self {
                Self::from(self.into_raw().to_next_multiple_of(alignment))
            }
            #[inline(always)]
            fn to_previous_multiple_of(self, alignment: usize) -> Self {
                Self::from(self.into_raw().to_previous_multiple_of(alignment))
            }
        }

        impl const AlignTo<$ty> for $ty {
            #[inline(always)]
            fn to_next_multiple_of(self, alignment: Self) -> Self {
                self.to_next_multiple_of(alignment.into_raw())
            }
            #[inline(always)]
            fn to_previous_multiple_of(self, alignment: Self) -> Self {
                self.to_previous_multiple_of(alignment.into_raw())
            }
        }
    };
}

impl_addr_ty!(VirtAddr);
impl_addr_ty!(PhysAddr);

impl VirtAddr {
    #[inline(always)]
    pub fn from_ptr<T: ?Sized>(value: *const T) -> Self {
        Self(value.addr())
    }

    #[inline(always)]
    pub const fn into_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    /// Returns the equalivent PhysAddr for the Page containing this VirtualAddr assuming it exists in the HHDM
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

#[const_trait]
pub trait AlignTo<Other>: Sized {
    /// Aligns (rounds) `self` to the next multiple of `alignment` aka align up
    ///
    /// for example: 1.to_next_multiple_of(2) == 2
    fn to_next_multiple_of(self, alignment: Other) -> Self;
    /// Aligns (rounds) `self` to the previous multiple of `alignment` aka align down
    ///
    /// for example: 3.to_previous_multiple_of(2) == 2
    fn to_previous_multiple_of(self, alignment: Other) -> Self;
}

#[const_trait]
pub trait AlignToPage: const AlignTo<usize> {
    #[inline(always)]
    /// Aligns (rounds) `self` to the next multiple of [`PAGE_SIZE`]
    ///
    /// for example: 0x100.to_next_page() == 0x1000 (4096)
    fn to_next_page(self) -> Self {
        self.to_next_multiple_of(PAGE_SIZE)
    }
    #[inline(always)]
    /// Aligns (rounds) `self` to the previous multiple of [`PAGE_SIZE`]
    ///
    /// for example: 0x2010.to_previous_page() == 0x2000 (4096*2)
    fn to_previous_page(self) -> Self {
        self.to_previous_multiple_of(PAGE_SIZE)
    }
}

macro_rules! impl_align_common {
    ($ty: ty, $from: ty) => {
        impl const AlignTo<$from> for $ty {
            #[inline(always)]
            fn to_next_multiple_of(self, alignment: $from) -> Self {
                let alignment = alignment as $ty;
                (self + alignment - 1) & !(alignment - 1)
            }
            #[inline(always)]
            fn to_previous_multiple_of(self, alignment: $from) -> Self {
                let alignment = alignment as $ty;
                self & !(alignment - 1)
            }
        }
    };

    ($ty: ty) => {
        impl_align_common!($ty, $ty);
    };
}

impl_align_common!(usize);
impl<T> const AlignToPage for T where T: const AlignTo<usize> {}

impl_align_common!(usize, u64);
impl_align_common!(usize, u32);
impl_align_common!(usize, u16);
impl_align_common!(u64);
impl_align_common!(u64, u32);
impl_align_common!(u64, u16);
impl_align_common!(u32);
impl_align_common!(u32, u16);
impl_align_common!(u16);

/// Copies from an address in a given page table to another address in the same page table
#[inline(always)]
pub fn userspace_copy_within(
    page_table: &mut PageTable,
    src_addr: VirtAddr,
    dest_addr: VirtAddr,
    size: usize,
) {
    let end_src_addr = src_addr + size;
    let end_dest_addr = dest_addr + size;

    let src_iter = Page::iter_pages(
        Page::containing_address(src_addr),
        Page::containing_address(end_src_addr + PAGE_SIZE),
    );

    let dest_iter = Page::iter_pages(
        Page::containing_address(dest_addr),
        Page::containing_address(end_dest_addr + PAGE_SIZE),
    );

    let pages_iter = src_iter.zip(dest_iter);
    let phys_addr_iter = pages_iter.map(|(src_page, dest_page)| {
        let src_frame = page_table
            .get_frame(src_page)
            .expect("attempt to copy from an unmapped page");
        let dest_frame = page_table
            .get_frame(dest_page)
            .expect("attempt to copy to an unmapped page");

        let (diff, to_copy) = if src_page.virt_addr() == src_addr.to_previous_page() {
            (
                src_addr - src_page.virt_addr(),
                src_addr - src_addr.to_next_page(),
            )
        } else if src_page.virt_addr() == end_src_addr.to_previous_page() {
            (0, src_page.virt_addr() - end_src_addr)
        } else {
            (0, PAGE_SIZE)
        };

        let src_phys_addr = src_frame.phys_addr() + diff;
        let dest_phys_addr = dest_frame.phys_addr() + diff;

        (src_phys_addr, dest_phys_addr, to_copy)
    });
    let pointers = phys_addr_iter.map(|(src, dest, size)| {
        (
            src.into_virt().into_ptr::<u8>() as *const u8,
            dest.into_virt().into_ptr::<u8>(),
            size,
        )
    });

    for (src, dest, size) in pointers {
        unsafe {
            dest.copy_from(src, size);
        }
    }
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
