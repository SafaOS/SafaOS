#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64::threading;

#[cfg(target_arch = "x86_64")]
pub use x86_64::{init_phase1, init_phase2};

#[cfg(target_arch = "x86_64")]
pub use x86_64::power;

#[cfg(target_arch = "x86_64")]
pub use x86_64::serial;

#[cfg(target_arch = "x86_64")]
pub use x86_64::utils;
