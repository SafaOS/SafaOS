pub mod buddy_allocator;
pub mod frame_allocator;
pub mod init;
pub mod paging;
pub mod vmm;

// FIXME: relays on unstable limine behaviour by assuming limine maps the HHDM at 0xffff800000000000 for x86_64
// The reason why I cannot do my own HHDM offset is because of the framebuffer which limine returns a virtual pointer to, so I don't know how I should map to a different address,
cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        /// The base offset of the HHDM in virtual memory
        pub const HHDM: VirtAddr = VirtAddr::from(0xffff800000000000);
    } else if #[cfg(target_arch = "aarch64")]{
        /// The base offset of the HHDM in virtual memory
        pub const HHDM: VirtAddr = VirtAddr::from(0xffff000000000000);
    } else {
        compile_error!("Setup HHDM base for your arch");
    }
}

use core::{
    fmt::{Debug, LowerHex},
    ops::{Add, AddAssign, Deref, DerefMut, Sub, SubAssign},
};

use paging::{PAGE_SIZE, Page, PageTable};
use serde::Serialize;

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

        const impl Add<usize> for $ty {
            type Output = $ty;
            #[inline(always)]
            fn add(self, rhs: usize) -> Self::Output {
                Self(self.0 + rhs)
            }
        }

        const impl Add<$ty> for $ty {
            type Output = $ty;
            #[inline(always)]
            fn add(self, rhs: $ty) -> Self::Output {
                self + rhs.0
            }
        }

        const impl AddAssign<usize> for $ty {
            #[inline(always)]
            fn add_assign(&mut self, rhs: usize) {
                *self = *self + rhs
            }
        }

        const impl Sub<$ty> for $ty {
            type Output = usize;
            #[inline(always)]
            fn sub(self, rhs: $ty) -> Self::Output {
                self.0 - rhs.0
            }
        }

        const impl Sub<usize> for $ty {
            type Output = Self;
            #[inline(always)]
            fn sub(self, rhs: usize) -> Self::Output {
                Self(self.0 - rhs)
            }
        }

        const impl SubAssign<usize> for $ty {
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

        const impl AlignTo<usize> for $ty {
            #[inline(always)]
            fn to_next_multiple_of(self, alignment: usize) -> Self {
                Self::from(self.into_raw().to_next_multiple_of(alignment))
            }
            #[inline(always)]
            fn to_previous_multiple_of(self, alignment: usize) -> Self {
                Self::from(self.into_raw().to_previous_multiple_of(alignment))
            }
        }

        const impl AlignTo<$ty> for $ty {
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

    /// Returns true if the address is in the lower half of the address space.
    pub const fn is_in_lower_half(self) -> bool {
        self.0 < (usize::MAX / 2)
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
    pub const fn into_virt(self) -> VirtAddr {
        VirtAddr(self.0 | HHDM.0)
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

pub const trait AlignTo<Other>: Sized {
    /// Aligns (rounds) `self` to the next multiple of `alignment` aka align up
    ///
    /// for example: 1.to_next_multiple_of(2) == 2
    fn to_next_multiple_of(self, alignment: Other) -> Self;
    /// Aligns (rounds) `self` to the previous multiple of `alignment` aka align down
    ///
    /// for example: 3.to_previous_multiple_of(2) == 2
    fn to_previous_multiple_of(self, alignment: Other) -> Self;
}

pub const trait AlignToPage: [const] AlignTo<usize> {
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
        const impl AlignTo<$from> for $ty {
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
const impl<T> AlignToPage for T where T: [const] AlignTo<usize> {}

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
pub fn pagetable_copy_within(
    page_table: &mut PageTable,
    src_addr: VirtAddr,
    dest_addr: VirtAddr,
    size: usize,
) {
    let end_src_addr = src_addr + size;
    let end_dest_addr = dest_addr + size;

    let src_iter = Page::iter_pages(
        Page::containing(src_addr),
        Page::containing(end_src_addr + PAGE_SIZE),
    );

    let dest_iter = Page::iter_pages(
        Page::containing(dest_addr),
        Page::containing(end_dest_addr + PAGE_SIZE),
    );

    let pages_iter = src_iter.zip(dest_iter);
    let phys_addr_iter = pages_iter.map(|(curr_src_page, curr_dest_page)| {
        let src_frame = page_table
            .get_frame_of(curr_src_page)
            .expect("attempt to copy from an unmapped page");
        let dest_frame = page_table
            .get_frame_of(curr_dest_page)
            .expect("attempt to copy to an unmapped page");

        let calc_within = |curr_page: VirtAddr, start_addr: VirtAddr, end_addr: VirtAddr| {
            if curr_page == start_addr.to_previous_page() {
                let offset_within = start_addr - curr_page;

                let to_next_page = PAGE_SIZE - offset_within;

                let to_end = end_addr - start_addr;

                let to_copy = core::cmp::min(to_next_page, to_end);

                (offset_within, to_copy)
            } else if curr_page == end_addr.to_previous_page() {
                (0, end_addr - curr_page)
            } else {
                (0, PAGE_SIZE)
            }
        };

        let (curr_src_diff, to_copy) = calc_within(curr_src_page.addr(), src_addr, end_src_addr);

        let (curr_dest_diff, _) = calc_within(curr_dest_page.addr(), dest_addr, end_dest_addr);

        let src_phys_addr = src_frame.phys_addr() + curr_src_diff;
        let dest_phys_addr = dest_frame.phys_addr() + curr_dest_diff;

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
pub fn copy_to_pagetable(page_table: &mut PageTable, addr: VirtAddr, obj: &[u8]) {
    let pages_required = obj.len().div_ceil(PAGE_SIZE) + 1;
    let mut copied = 0;
    let mut to_copy = obj.len();

    for i in 0..pages_required {
        let page = Page::containing(addr + copied);
        let diff = if i == 0 { addr - page.addr() } else { 0 };
        let will_copy = if (to_copy + diff) >= PAGE_SIZE {
            PAGE_SIZE - diff
        } else {
            to_copy
        };

        if will_copy == 0 {
            return;
        }

        let Some(frame) = page_table.get_frame_of(page) else {
            panic!("attempt to copy to an unmapped page: {page:?}");
        };

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
