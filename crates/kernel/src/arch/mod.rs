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
        pub mod unsupported;
        use unsupported as arch;
    }
}

/// Contains everything related to threading, such as code for context switching
pub mod threading {
    pub use super::arch::threading::{
        CPUStatus, cpu_local_storage_ptr, cpu_local_storages, init_cpus, invoke_context_switch,
        restore_cpu_status,
    };
}

pub use arch::{disable_interrupts, enable_interrupts, flush_cache, hlt, init_phase1, init_phase2};

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
    pub use super::arch::registers::{MPIDR, StackFrame};
}

pub mod pci {
    pub use super::arch::pci::{build_msi_addr, build_msi_data, init};
}

pub mod interrupts {
    pub use super::arch::interrupts::{IRQS, register_irq_handler};
}

pub use arch::paging;
