//! Kernel's runtime stuff, this provides abstraction to kernel I/O and scheduling primitives.

pub mod driver;
pub mod interrupt;
pub mod queue;

pub use queue::*;

#[cfg(test)]
pub(crate) mod test_util;
