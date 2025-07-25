use crate::ffi::NotZeroable;

/// An FFI Safe [Option]-like type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(C, u8)]
pub enum COption<T> {
    #[default]
    None = 0,
    Some(T) = 1,
}

impl<T> From<Option<T>> for COption<T> {
    #[inline(always)]
    fn from(value: Option<T>) -> Self {
        match value {
            None => Self::None,
            Some(x) => Self::Some(x),
        }
    }
}

impl<T> From<COption<T>> for Option<T> {
    #[inline(always)]
    fn from(value: COption<T>) -> Self {
        match value {
            COption::None => None,
            COption::Some(x) => Some(x),
        }
    }
}

/// Represents an Option where a zero value is considered None.
///
/// Wraps an FFI type that SHOULD be non zeroable, and provides a safe way to handle zero values as None
///
/// The reason why i used "SHOULD" is because that type may be zeroed if passed from a foreign callsite so extra handling is required, this type makes it safe for it to be zeroed.
#[derive(Clone, Copy, Hash, Default)]
#[repr(transparent)]
pub struct OptZero<T: NotZeroable>(T);

impl<T: NotZeroable> OptZero<T> {
    /// Creates a new `OptZero` that represents a Some value from a value.
    pub const fn some(x: T) -> Self {
        Self(x)
    }
    /// Creates a new `OptZero` that represents a None value.
    pub const fn none() -> Self {
        Self(unsafe { core::mem::zeroed() })
    }
    /// Creates a new `OptZero` from an `Option`.
    pub fn from_option(opt: Option<T>) -> Self {
        match opt {
            None => Self::none(),
            Some(x) => Self::some(x),
        }
    }
    /// Converts this `OptZero` into an `Option`.
    pub fn into_option(self) -> Option<T> {
        match self.0.is_zero() {
            false => Some(self.0),
            true => None,
        }
    }
    /// Returns a reference to the contained value or `None` if the value is zeroed.
    pub fn as_option(&self) -> Option<&T> {
        match self.0.is_zero() {
            false => Some(&self.0),
            true => None,
        }
    }
    /// Maps a `OptZero` to another `OptZero` using a function.
    pub fn map<F, U>(self, f: F) -> OptZero<U>
    where
        F: FnOnce(T) -> U,
        U: NotZeroable,
    {
        match self.0.is_zero() {
            false => OptZero::some(f(self.0)),
            true => OptZero::none(),
        }
    }
}

impl<T: NotZeroable + PartialEq> PartialEq for OptZero<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        (self.0.is_zero() && other.0.is_zero()) || (self.0 == other.0)
    }
}

impl<T: NotZeroable + Eq> Eq for OptZero<T> {}

impl<T: NotZeroable> From<OptZero<T>> for Option<T> {
    #[inline(always)]
    fn from(value: OptZero<T>) -> Self {
        value.into_option()
    }
}

impl<T: NotZeroable> From<Option<T>> for OptZero<T> {
    #[inline(always)]
    fn from(value: Option<T>) -> Self {
        Self::from_option(value)
    }
}
