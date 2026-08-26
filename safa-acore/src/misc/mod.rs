mod frame;
mod memory;
mod page;

use core::ops::{Div, Mul};

pub use frame::*;
pub use memory::*;
pub use page::*;

#[inline(always)]
pub const fn to_previous_multiple_of<T: Copy>(x: T, alignment: T) -> T
where
    T: [const] Mul<Output = T> + [const] Div<Output = T>,
{
    (x / alignment) * alignment
}
