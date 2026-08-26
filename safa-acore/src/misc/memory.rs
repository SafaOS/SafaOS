use core::fmt::Debug;
use core::ops::{Add, AddAssign, Deref, DerefMut, Sub, SubAssign};

pub const KIB_1: usize = 1024;
pub const KIB_4: usize = KIB_1 * 4;
pub const PAGE_SIZE: usize = KIB_4;

// FIXME: Implementition of serialize should serialize as hex string because memory addresses don't fit in json's int
/// A virtual memory address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(transparent)]
pub struct VirtAddr(usize);

/// A physical memory address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(transparent)]
pub struct PhysAddr(usize);

impl Debug for VirtAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}

impl Debug for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PhysAddr({:#x})", self.0)
    }
}

macro_rules! impl_addr_ty {
    ($ty: ty) => {
        impl $ty {
            #[inline(always)]
            /// Returns a null address.
            pub const fn null() -> Self {
                Self(0)
            }

            #[inline(always)]
            /// Interprets a usize value as an address.
            pub const fn new(value: usize) -> Self {
                Self(value)
            }

            #[inline(always)]
            #[allow(unused)]
            /// Used for bitfields alias for [`Self::raw`].
            pub const fn into_bits(self) -> usize {
                self.raw()
            }

            #[inline(always)]
            /// Returns this address as a usize.
            pub const fn raw(self) -> usize {
                self.0
            }

            #[inline(always)]
            /// Returns the page number of this address
            pub const fn page_num(self) -> usize {
                self.raw() / $crate::misc::PAGE_SIZE
            }

            #[inline(always)]
            /// Returns the previous address that is aligned down to `x` before this.
            pub const fn prev_multiple_of(self, x: usize) -> Self {
                Self($crate::misc::to_previous_multiple_of(self.raw(), x))
            }

            #[inline(always)]
            /// Returns the next address that is aligned up to `x` after this.
            pub const fn next_multiple_of(self, x: usize) -> Self {
                Self(self.raw().next_multiple_of(x))
            }

            #[inline(always)]
            /// Returns the address of the page that contains this address.
            pub const fn prev_page(self) -> Self {
                self.prev_multiple_of($crate::misc::PAGE_SIZE)
            }

            #[inline(always)]
            /// Returns the address aligned up to the next page after the [`Self::previous_page`] if [`Self::previous_page(self)`] != self.
            pub const fn next_page(self) -> Self {
                self.next_multiple_of($crate::misc::PAGE_SIZE)
            }

            #[inline(always)]
            #[allow(unused)]
            /// Used for bitfields alias for [`Self::new`].
            pub const fn from_bits(bits: usize) -> Self {
                Self::new(bits)
            }
        }

        impl From<usize> for $ty {
            #[inline(always)]
            fn from(value: usize) -> Self {
                Self::new(value)
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
    };
}

impl_addr_ty!(VirtAddr);
impl_addr_ty!(PhysAddr);

impl VirtAddr {
    #[inline(always)]
    /// Converts the address part of a given pointer `value` into a virtual addresses.
    pub fn from_ptr<T: ?Sized>(value: *const T) -> Self {
        Self(value.addr())
    }

    #[inline(always)]
    /// Interpreters this address as a pointer to a given type T.
    pub const fn into_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    /// Returns true if the address is in the lower half of the address space.
    #[inline(always)]
    pub const fn is_in_lower_half(self) -> bool {
        self.0 < (usize::MAX / 2)
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
