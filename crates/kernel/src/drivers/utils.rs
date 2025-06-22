// Thanks to optimizations I have to perform voliatile reads and writes otherwise it doesn't work
// safe because it is a reference anyways

/// Performs a safe volitate read to a structure field
#[macro_export]
macro_rules! read_ref {
    ($ref: expr) => {
        unsafe { core::ptr::read_volatile(&raw const $ref) }
    };
}
pub use read_ref;

/// Performs a safe volitate write to a structure's field
#[macro_export]
macro_rules! write_ref {
    ($ref: expr, $value: expr) => {
        unsafe { core::ptr::write_volatile(&raw mut $ref, $value) }
    };
}

pub use write_ref;
