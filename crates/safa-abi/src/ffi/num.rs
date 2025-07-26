use crate::ffi::NotZeroable;
use crate::ffi::option::OptZero;

use core::num::NonZero;

/// Describes a trait for some types that are valid when passed as a zero
///
/// mainly for usage with [`ShouldNotBeZero`]
pub trait ZeroableThing: Sized {
    const ZERO: Self;
    fn is_zero(&self) -> bool;
}

macro_rules! impl_zeroable_int {
    ($ty: ty) => {
        impl ZeroableThing for $ty {
            const ZERO: Self = 0;
            fn is_zero(&self) -> bool {
                *self == Self::ZERO
            }
        }

        impl From<NonZero<$ty>> for ShouldNotBeZero<$ty> {
            fn from(value: NonZero<$ty>) -> Self {
                ShouldNotBeZero(value.into())
            }
        }

        impl From<OptZero<ShouldNotBeZero<$ty>>> for Option<NonZero<$ty>> {
            fn from(value: OptZero<ShouldNotBeZero<$ty>>) -> Self {
                match value.into_option() {
                    Option::Some(value) => Some(unsafe { NonZero::new_unchecked(value.value()) }),
                    Option::None => None,
                }
            }
        }
    };
}

impl_zeroable_int!(u8);
impl_zeroable_int!(u16);
impl_zeroable_int!(u32);
impl_zeroable_int!(u64);
impl_zeroable_int!(u128);
impl_zeroable_int!(usize);
impl_zeroable_int!(i8);
impl_zeroable_int!(i16);
impl_zeroable_int!(i32);
impl_zeroable_int!(i64);
impl_zeroable_int!(i128);
impl_zeroable_int!(isize);

#[repr(transparent)]
/// Describes something that is valid when passed as a zero but SHOULDN'T BE a zero
///
/// mainly for usage with FFI and [`OptZero`] so that if it was passed as a zero it would be treated as a None value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShouldNotBeZero<T: ZeroableThing>(T);

impl<T: ZeroableThing> ShouldNotBeZero<T> {
    pub fn new(value: T) -> Option<Self> {
        if value.is_zero() {
            None
        } else {
            Some(ShouldNotBeZero(value))
        }
    }

    pub const unsafe fn new_unchecked(value: T) -> Self {
        ShouldNotBeZero(value)
    }

    pub const fn value_ref(&self) -> &T {
        &self.0
    }
}

impl<T: ZeroableThing + Copy> ShouldNotBeZero<T> {
    pub const fn value(&self) -> T {
        self.0
    }
}

impl<T: ZeroableThing> NotZeroable for ShouldNotBeZero<T> {
    fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
}
