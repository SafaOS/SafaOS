//! Architecture specific code,
//! this module contains everything that would make a difference between architectures such i   nitilization and handling context switching
use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        pub mod x86_64;
        use x86_64 as arch;
    }  else if #[cfg(target_arch = "aarch64")] {
        pub mod aarch64;
        use aarch64 as arch;
    }
    else {
        compile_error!("unsupported architecture (target_arch unsupported)");
    }
}

/// Contains everything related to threading, such as code for context switching
pub mod threading {
    pub use super::arch::threading::{
        CPUStatus, cpu_local_storage_ptr, cpu_local_storages, init_cpus, invoke_context_switch,
        restore_cpu_status,
    };
}

pub use arch::{flush_cache, halt_all, hlt, init_phase1, init_phase2, without_interrupts};

pub mod power {
    pub use super::arch::power::{reboot, shutdown};
}

pub mod serial {
    pub use super::arch::serial::{_serial, SERIAL, Serial};
}

pub mod utils {
    #[allow(unused_imports)]
    pub use super::arch::utils::{CPU_INFO, time_ms, time_us};
}

pub mod registers {
    pub use super::arch::registers::{CPUID, StackFrame};
}

pub mod pci {
    pub use super::arch::pci::{build_msi_addr, build_msi_data, init};
}

pub mod interrupts {
    pub use super::arch::interrupts::{IRQS, register_irq_handler};
}

pub use arch::paging;
use lazy_static::lazy_static;

lazy_static! {
    static ref CPU_COUNT: u32 = {
        use crate::limine::MP_RESPONSE;
        (MP_RESPONSE.cpus().len()) as u32
    };
}
/// Returns the number of available CPUs
#[inline]
pub fn available_cpus() -> u32 {
    *CPU_COUNT
}
