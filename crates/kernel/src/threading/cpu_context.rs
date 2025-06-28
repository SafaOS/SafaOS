//! Defines the CPU Context for the smallest unit of execution in the system that is a thread.

use crate::arch::threading::CPUStatus;

/// Context ID, a unique identifier for a thread.
type Cid = u32;

pub enum ContextStatus {
    Runnable,
    Sleeping(u64),
    AwaitingCleanup,
}

pub struct Context {
    id: Cid,
    status: ContextStatus,
    cpu_status: CPUStatus,
}

impl Context {
    pub const fn cid(&self) -> Cid {
        self.id
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

    pub fn exit(&mut self) {
        self.status = ContextStatus::AwaitingCleanup;
    }
}
