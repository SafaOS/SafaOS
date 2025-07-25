//! Defines the current smallest unit of execution in the scheduler (a Task) that is a thread.

use core::{cell::UnsafeCell, sync::atomic::AtomicBool};

use crate::{
    arch::threading::CPUStatus,
    debug,
    memory::proc_mem_allocator::TrackedAllocation,
    process::{Pid, Process},
    scheduler::CPULocalStorage,
    time,
    utils::locks::SpinMutex,
};

pub mod current;

/// Thread ID, a unique identifier for a thread.
pub type Tid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContextPriority {
    Low,
    Medium,
    High,
}

impl ContextPriority {
    /// Returns the number of timeslices a thread with this priority should be given.
    pub const fn timeslices(&self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Medium => 3,
            Self::High => 5,
        }
    }
}

impl From<RawContextPriority> for ContextPriority {
    fn from(value: RawContextPriority) -> Self {
        match value {
            RawContextPriority::Default => Self::Medium,
            RawContextPriority::High => Self::High,
            RawContextPriority::Medium => Self::Medium,
            RawContextPriority::Low => Self::Low,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BlockedReason {
    /// The thread is sleeping until [`.0`] ms of boot time is reached
    SleepingUntil(u128),
    WaitingForProcess(Arc<Process>),
    WaitingForThread(Arc<Thread>),
    WaitOnFutex {
        addr: *mut u32,
        value: u32,
        timeout_wake_at: u128,
    },
}

impl BlockedReason {
    pub fn block_lifted(&self) -> bool {
        match self {
            Self::SleepingUntil(n) => time!(ms) as u128 >= *n,
            Self::WaitingForProcess(process) => !process.is_alive(),
            Self::WaitingForThread(thread) => thread.is_dead(),
            Self::WaitOnFutex {
                timeout_wake_at, ..
            } => time!(ms) as u128 >= *timeout_wake_at,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ContextStatus {
    Running,
    Runnable,
    Blocked(BlockedReason),
}

impl ContextStatus {
    pub const fn is_running(&self) -> bool {
        match self {
            Self::Running => true,
            _ => false,
        }
    }

    pub fn try_lift_futex(&mut self, target_addr: *mut u32) -> bool {
        match *self {
            Self::Blocked(BlockedReason::WaitOnFutex { addr, value, .. })
                if target_addr == addr && unsafe { *addr != value } =>
            {
                *self = Self::Runnable;
                true
            }
            _ => false,
        }
    }
}

use alloc::{boxed::Box, sync::Arc};
use safa_abi::process::RawContextPriority;

#[derive(Debug, Clone)]
/// A node representing a Thread in a thread queue
pub struct ThreadNode {
    inner: Arc<Thread>,
    pub next: Option<Box<ThreadNode>>,
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
    id: Tid,
    priority: ContextPriority,
    status: SpinMutex<ContextStatus>,
    context: UnsafeCell<Option<Context>>,

    is_dead: AtomicBool,
    is_removed: AtomicBool,
    parent_process: Arc<Process>,
}

impl Thread {
    pub fn new(
        cid: Tid,
        cpu_status: CPUStatus,
        parent_process: &Arc<Process>,
        priority: ContextPriority,
        tracked_allocations: heapless::Vec<TrackedAllocation, 3>,
    ) -> Self {
        Self {
            id: cid,
            priority,
            status: SpinMutex::new(ContextStatus::Runnable),
            context: UnsafeCell::new(Some(Context::new(cpu_status, tracked_allocations))),
            is_dead: AtomicBool::new(false),
            is_removed: AtomicBool::new(false),
            parent_process: parent_process.clone(),
        }
    }

    pub const fn priority(&self) -> ContextPriority {
        self.priority
    }

    pub const fn process(&self) -> &Arc<Process> {
        &self.parent_process
    }

    pub const unsafe fn context(&self) -> Option<&mut Context> {
        unsafe { &mut *self.context.get() }.as_mut()
    }

    pub const unsafe fn context_unchecked(&self) -> &mut Context {
        unsafe { self.context().unwrap_unchecked() }
    }

    pub const fn tid(&self) -> Tid {
        self.id
    }

    pub fn is_dead(&self) -> bool {
        self.is_dead.load(core::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_removed(&self) -> bool {
        self.is_removed.load(core::sync::atomic::Ordering::Acquire)
    }

    pub fn mark_removed(&self) {
        unsafe {
            *self.context.get() = None;
        }
        self.is_removed
            .store(true, core::sync::atomic::Ordering::Release);
    }

    pub fn mark_dead(&self, process_dead: bool) {
        self.is_dead
            .store(true, core::sync::atomic::Ordering::SeqCst);

        debug!(
            Process,
            "Thread {}:{} ({}) THREAD EXITED, process dead: {process_dead}",
            self.process().pid(),
            self.tid(),
            self.process().name(),
        );
    }

    pub fn kill_thread(&self, exit_code: usize) {
        let process = &self.parent_process;
        let _state = process.state_mut();

        let process_dead = process
            .context_count
            .fetch_sub(1, core::sync::atomic::Ordering::SeqCst)
            <= 1;

        self.mark_dead(process_dead);

        if process_dead {
            drop(_state);
            process.kill(exit_code, None);
        }
    }

    pub fn status_mut<'a>(&'a self) -> spin::MutexGuard<'a, ContextStatus> {
        self.status.lock()
    }

    /// Should only be called by the current thread or the scheduler or on a sleeping thread
    pub fn set_status(&self, status: ContextStatus) {
        *self.status.lock() = status;
    }

    /// Should only be called by the current thread
    pub fn sleep_for_ms(&self, ms: u64) {
        self.set_status(ContextStatus::Blocked(BlockedReason::SleepingUntil(
            (time!(ms) as u128) + ms as u128,
        )));
    }

    /// Should only be called by the current thread
    pub fn wait_for_process(&self, process: Arc<Process>) {
        self.set_status(ContextStatus::Blocked(BlockedReason::WaitingForProcess(
            process,
        )));
    }

    /// Should only be called by the current thread
    pub fn wait_for_thread(&self, thread: Arc<Thread>) {
        self.set_status(ContextStatus::Blocked(BlockedReason::WaitingForThread(
            thread,
        )));
    }

    /// Should only be called by the current thread
    pub fn wait_for_futex(&self, addr: *mut u32, with_value: u32, timeout_ms: u64) {
        self.set_status(ContextStatus::Blocked(BlockedReason::WaitOnFutex {
            addr,
            value: with_value,
            timeout_wake_at: time!(ms) as u128 + timeout_ms as u128,
        }));
    }
}

#[derive(Debug)]
pub struct Context {
    cpu_status: CPUStatus,
    _tracked_allocations: heapless::Vec<TrackedAllocation, 3>,
}

impl Context {
    pub const fn set_cpu_status(&mut self, status: CPUStatus) {
        self.cpu_status = status;
    }

    pub unsafe fn cpu_status(&mut self) -> core::ptr::NonNull<CPUStatus> {
        unsafe { core::ptr::NonNull::new_unchecked(&mut self.cpu_status) }
    }

    pub(super) fn new(
        cpu_status: CPUStatus,
        tracked_allocations: heapless::Vec<TrackedAllocation, 3>,
    ) -> Self {
        Context {
            cpu_status,
            _tracked_allocations: tracked_allocations,
        }
    }
}

/// Returns the current thread, that is the thread executing this code right now.
pub fn current() -> Arc<Thread> {
    CPULocalStorage::get().current_thread()
}

/// Returns the current process ID, that is the ID of the process executing this code right now.
///
/// faster than [`current()`]`.process().pid()`
pub fn current_pid() -> Pid {
    CPULocalStorage::get().current_thread_ref().process().pid()
}
