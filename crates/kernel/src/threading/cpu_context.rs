//! Defines the CPU Context for the smallest unit of execution in the system that is a thread.

use core::{cell::UnsafeCell, sync::atomic::AtomicBool};

use crate::{arch::threading::CPUStatus, debug, threading::task::Task, time};

/// Context ID, a unique identifier for a thread.
pub type Cid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStatus {
    Running,
    Runnable,
    /// The thread is sleeping for a specified number of milliseconds.
    Sleeping(u64),
}

use alloc::{boxed::Box, sync::Arc};
pub use safa_utils::abi::raw::processes::ContextPriority;

#[derive(Debug, Clone)]
/// A node representing a Thread in a thread queue
pub struct ThreadNode {
    inner: Arc<Thread>,
    pub(super) next: Option<Box<ThreadNode>>,
}

impl ThreadNode {
    pub const fn new(thread: Arc<Thread>) -> Self {
        Self {
            inner: thread,
            next: None,
        }
    }

    pub const fn thread(&self) -> &Arc<Thread> {
        &self.inner
    }

    /// Given a node that is a head of the thread list, make this thread the head instead
    pub fn push_front(this: &mut Box<Self>, thread: Arc<Thread>) {
        let node = ThreadNode::new(thread);
        let old_node = core::mem::replace(this, Box::new(node));
        // now this is the new node
        this.next = Some(old_node);
    }
}

#[derive(Debug)]
pub struct Thread {
    context: UnsafeCell<Context>,
    is_dead: AtomicBool,
    is_removed: AtomicBool,
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
            is_removed: AtomicBool::new(false),
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
        self.is_dead.load(core::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_removed(&self) -> bool {
        self.is_removed.load(core::sync::atomic::Ordering::Acquire)
    }

    pub fn mark_removed(&self) {
        self.is_removed
            .store(true, core::sync::atomic::Ordering::Release);
    }

    pub fn mark_dead(&self, task_dead: bool) {
        self.is_dead
            .store(true, core::sync::atomic::Ordering::SeqCst);

        debug!(
            Task,
            "Task {} ({}) THREAD EXITED thread CID: {}, task dead: {task_dead}",
            self.task().pid(),
            self.task().name(),
            unsafe { self.context().cid() },
        );
    }

    pub fn kill_thread(&self, exit_code: usize) {
        let task = &self.parent_task;
        let _state = task.state_mut();

        let task_dead = task
            .context_count
            .fetch_sub(1, core::sync::atomic::Ordering::SeqCst)
            <= 1;

        self.mark_dead(task_dead);

        if task_dead {
            drop(_state);
            task.kill(exit_code, None);
        }
    }
}

#[derive(Debug, Clone)]
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
