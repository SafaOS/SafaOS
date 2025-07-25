//! FFI bindings for SafaOS's ABI
//!
//! for example exports [`RawSlice<T>`] which is an FFI safe alternative to `&[T]`

pub mod option;
pub mod ptr;
pub mod slice;
pub mod str;

/// Defines a trait for types that are invalid when passed as a zero
pub trait NotZeroable: Sized {
    fn is_zero(&self) -> bool;
}
