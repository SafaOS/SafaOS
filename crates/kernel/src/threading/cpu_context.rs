//! Defines the CPU Context for the smallest unit of execution in the system that is a thread.

use crate::{arch::threading::CPUStatus, time};

/// Context ID, a unique identifier for a thread.
pub type Cid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStatus {
    Runnable,
    /// The thread is sleeping for a specified number of milliseconds.
    Sleeping(u64),
}

#[derive(Debug)]
pub struct Context {
    id: Cid,
    status: ContextStatus,
    cpu_status: CPUStatus,
}

impl Context {
    pub const fn cid(&self) -> Cid {
        self.id
    }

    pub const fn status(&self) -> ContextStatus {
        self.status
    }

    pub const fn set_status(&mut self, status: ContextStatus) {
        self.status = status;
    }

    pub fn sleep_for_ms(&mut self, ms: u64) {
        self.status = ContextStatus::Sleeping(time!(ms) + ms);
    }

    pub const fn set_cpu_status(&mut self, status: CPUStatus) {
        self.cpu_status = status;
    }

    pub fn cpu_status(&mut self) -> core::ptr::NonNull<CPUStatus> {
        unsafe { core::ptr::NonNull::new_unchecked(&mut self.cpu_status) }
    }

    pub(super) fn new(id: Cid, cpu_status: CPUStatus) -> Self {
        Context {
            status: ContextStatus::Runnable,
            id,
            cpu_status,
        }
    }
}
