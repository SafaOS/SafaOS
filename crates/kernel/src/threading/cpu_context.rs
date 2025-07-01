//! Defines the CPU Context for the smallest unit of execution in the system that is a thread.

use core::{cell::UnsafeCell, sync::atomic::AtomicBool};

use crate::{arch::threading::CPUStatus, debug, threading::task::Task, time};

/// Context ID, a unique identifier for a thread.
pub type Cid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStatus {
    Runnable,
    /// The thread is sleeping for a specified number of milliseconds.
    Sleeping(u64),
}

use alloc::sync::Arc;
pub use safa_utils::abi::raw::processes::ContextPriority;

pub struct Thread {
    context: UnsafeCell<Context>,
    is_dead: AtomicBool,
    parent_task: Arc<Task>,
}

impl Thread {
    pub fn new(
        cid: Cid,
        cpu_status: CPUStatus,
        parent_task: &Arc<Task>,
        priority: ContextPriority,
    ) -> Self {
        Self {
            context: UnsafeCell::new(Context::new(cid, cpu_status, priority)),
            is_dead: AtomicBool::new(false),
            parent_task: parent_task.clone(),
        }
    }

    pub fn task(&self) -> &Arc<Task> {
        &self.parent_task
    }

    pub const unsafe fn context(&self) -> &mut Context {
        unsafe { &mut *self.context.get() }
    }

    pub fn is_dead(&self) -> bool {
        self.is_dead.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub fn mark_dead(&self) {
        self.is_dead
            .store(true, core::sync::atomic::Ordering::SeqCst);
    }

    pub fn kill_thread(&self, exit_code: usize) {
        let task = &self.parent_task;
        let _state = task.state_mut();

        self.mark_dead();

        let task_dead = task
            .context_count
            .fetch_sub(1, core::sync::atomic::Ordering::SeqCst)
            == 0;

        let cid = unsafe { self.context().cid() };
        debug!(
            Task,
            "Task {} ({}) THREAD EXITED thread CID: {}, exit code: {}, task dead: {}",
            task.pid(),
            task.name(),
            cid,
            exit_code,
            task_dead
        );

        if task_dead {
            drop(_state);
            task.kill(exit_code, None);
        }
    }
}

#[derive(Debug)]
pub struct Context {
    id: Cid,

    priority: ContextPriority,

    status: ContextStatus,
    cpu_status: CPUStatus,
}

impl Context {
    pub const fn priority(&self) -> ContextPriority {
        self.priority
    }

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

    pub unsafe fn cpu_status(&mut self) -> core::ptr::NonNull<CPUStatus> {
        unsafe { core::ptr::NonNull::new_unchecked(&mut self.cpu_status) }
    }

    pub(super) fn new(id: Cid, cpu_status: CPUStatus, priority: ContextPriority) -> Self {
        Context {
            status: ContextStatus::Runnable,
            id,
            cpu_status,
            priority,
        }
    }
}
