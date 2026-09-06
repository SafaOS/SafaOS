#[cfg(target_arch = "aarch64")]
pub mod aarch64;
#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as inner;

#[cfg(target_arch = "aarch64")]
pub use aarch64 as inner;

pub mod acpi;
pub(crate) mod boot;
pub mod misc;
pub mod paging;
pub mod registers;
pub mod timers;
pub use misc::*;

pub(crate) mod serial;
