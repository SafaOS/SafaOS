//! Kernel's async runtime stuff.

pub mod queue;

pub use queue::*;

#[cfg(test)]
pub(crate) mod test_util;
