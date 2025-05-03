//! Architecture specific code,
//! this module contains everything that would make a difference between architectures such initilization and handling context switching
#[cfg(target_arch = "x86_64")]
pub mod x86_64;

/// Contains everything related to threading, such as code for context switching
pub mod threading {
    #[cfg(target_arch = "x86_64")]
    pub use super::x86_64::threading::{restore_cpu_status, CPUStatus};
}

#[cfg(target_arch = "x86_64")]
pub use x86_64::{init_phase1, init_phase2};

pub mod power {
    #[cfg(target_arch = "x86_64")]
    pub use super::x86_64::power::{reboot, shutdown};
}

pub mod serial {
    #[cfg(target_arch = "x86_64")]
    pub use super::x86_64::serial::{Serial, SERIAL};
}

pub mod utils {
    #[cfg(target_arch = "x86_64")]
    #[allow(unused_imports)]
    pub use super::x86_64::utils::{time, CPU_INFO};
}
