//! Defines the current smallest unit of execution in the scheduler (a Task) that is a thread.

use core::{
    cell::UnsafeCell,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{
    arch::threading::CPUStatus,
    debug, eve,
    memory::proc_mem_allocator::TrackedAllocation,
    process::{Pid, Process},
    scheduler::Scheduler,
    time,
    utils::locks::{SpinLock, SpinLockGuard},
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
    WaitingForThread(ArcThread),
    WaitOnFutex {
        addr: *const AtomicU32,
        value: u32,
        timeout_wake_at: u128,
    },
    BlockedForever,
}

impl BlockedReason {
    pub fn block_lifted(&self) -> bool {
        match self {
            Self::SleepingUntil(n)
            | Self::WaitOnFutex {
                timeout_wake_at: n, ..
            } => time!(ms) as u128 >= *n,
            Self::WaitingForProcess(process) => !process.is_alive(),
            Self::WaitingForThread(thread) => thread.is_dead(),
            Self::BlockedForever => false,
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

    pub fn try_lift_futex(&mut self, target_addr: *const AtomicU32) -> bool {
        match *self {
            Self::Blocked(BlockedReason::WaitOnFutex { addr, value, .. })
                if target_addr == addr && unsafe { (*addr).load(Ordering::SeqCst) != value } =>
            {
                *self = Self::Runnable;
                true
            }
            _ => false,
        }
    }
}

use alloc::sync::Arc;
use safa_abi::process::RawContextPriority;

/// A shared reference to a Thread, once dropped, removes itself from the thread list
#[derive(Debug, Clone)]
pub struct ArcThread(Arc<Thread>);

impl ArcThread {
    pub fn new(thread: Thread) -> Self {
        Self(Arc::new(thread))
    }
    /// Remove this thread from the thread list
    /// # Safety
    /// If the thread is the current thread, this function must be called without interrupts on
    unsafe fn remove_self(&self) {
        let Some(scheduler) = (unsafe { &*self.scheduler.get() }) else {
            panic!("Attempted to remove a thread that isn't associated with a scheduler")
        };

        let scheduler = unsafe { scheduler.as_ref() };
        let is_current = {
            let curr_thread = scheduler.current_thread_ref();
            // TODO: maybe we shouldn't identify threads by PID:TID combination, but by a unique identifier
            let curr_pid = curr_thread.process().pid();
            let curr_tid = curr_thread.tid();
            let this_pid = self.process().pid();
            let this_tid = self.tid();

            curr_pid == this_pid && curr_tid == this_tid
        };

        if !is_current {
            // NOTE:
            // we have 2 guranatees here
            // 1. The thread that we are trying to remove is not going to be accessed once we unblock the scheduler
            // 2. The Scheduler is blocked until the thread is completely removed
            // meaning that we don't have to worry about keeping a dangling next pointer
            // However in case we are the current thread, we will be accessed the next time we yield so we need to keep the next pointer
            // but it is safe to do so because no other thread should be removed or switched to during the removal of self
            self.block_forever();
        }

        /* ensures no other thread is going to be removed or switched to during this operation */
        let mut head_thread = scheduler.head_thread.lock();

        let next = unsafe { self.0.next_mut() };
        let prev = unsafe { self.0.prev_mut() };

        match (&*prev, &*next) {
            (None, None) => unreachable!("Attempted to remove an orphan thread"),
            (Some(prev), Some(next)) => {
                unsafe { *next.prev_mut() = Some(prev.clone()) };
                unsafe { *prev.next_mut() = Some(next.clone()) };
            }
            (Some(prev), None) => {
                unsafe { *prev.next_mut() = None };
            }
            (None, Some(next)) => {
                unsafe { *next.prev_mut() = None };
                *head_thread = next.clone();
            }
        }

        if is_current {
            self.set_status(ContextStatus::Blocked(BlockedReason::BlockedForever));
            eve::schedule_thread_cleanup(self.clone(), scheduler.context_switches_count_ref());
        } else {
            // the thread isn't running we can drop it now
            unsafe { self.cleanup() };
        }

        scheduler.sub_thread_count();
    }

    /// Assuming this is the head thread, makes `new_head` the new Thread head, adding it to the thread queue
    /// self becomes the new head thread
    /// # Safety
    /// self must be the head thread
    /// the caller must hold a lock on the scheduler
    pub unsafe fn add_to_head_thread(&mut self, new_head: ArcThread) {
        {
            let this_prev = unsafe { self.prev_mut() };
            debug_assert!(this_prev.is_none());

            let new_head_next = unsafe { new_head.next_mut() };
            debug_assert!(new_head_next.is_none());

            *new_head_next = Some(self.clone());
            *this_prev = Some(new_head.clone());
            unsafe {
                *new_head.scheduler.get() = *self.scheduler.get();
            }
        }

        *self = new_head;
    }

    /// Kills the thread without removing it from the process list,
    /// remove the thread from the Scheduler's task list
    /// # Safety
    /// The caller must remove the thread from the parent process's thread list.
    /// If this was called from the current thread, the caller must run it without interrupts.
    /// If this was the last thread in the process, the process must be killed by the caller.
    pub unsafe fn soft_kill(&self, process_dead: bool) {
        unsafe {
            self.remove_self();
        }
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

    /// Kills the thread removing it from the parent process's thread list unlike [`soft_kill`],
    /// also handles killing the process if it was the last thread and running without interrupts.
    ///
    /// # Safety
    /// The caller must handle the case that this is the current thread carefully, interrupts must be disabled and all caller resources shall be dropped.
    pub unsafe fn kill(&self, exit_code: usize) {
        let process = &self.parent_process;
        let process_dead = process
            .context_count
            .fetch_sub(1, core::sync::atomic::Ordering::SeqCst)
            <= 1;

        unsafe {
            self.soft_kill(process_dead);

            if process_dead {
                process.kill(exit_code, None);
            }
        }
    }
}

impl Drop for ArcThread {
    #[track_caller]
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) <= 1 {
            assert!(
                self.is_dead(),
                "Attempt to drop last reference of a thread that has not been killed, thread ID: {}, thread parent's ID: {}",
                self.tid(),
                self.parent_process.pid()
            );
        }
    }
}

impl Deref for ArcThread {
    type Target = Arc<Thread>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub struct Thread {
    id: Tid,
    priority: ContextPriority,
    status: SpinLock<ContextStatus>,
    context: UnsafeCell<Option<Context>>,

    is_dead: AtomicBool,
    is_removed: AtomicBool,
    parent_process: Arc<Process>,

    /// The scheduler that this thread belongs to.
    /// null until scheduled
    pub scheduler: UnsafeCell<Option<NonNull<Scheduler>>>,
    // For safety we have to follow 2 rules:
    // 1. reads must be performed by the scheduler
    // 2. writes must be performed with the scheduler's lock held
    next: UnsafeCell<Option<ArcThread>>,
    prev: UnsafeCell<Option<ArcThread>>,
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
            status: SpinLock::new(ContextStatus::Runnable),
            context: UnsafeCell::new(Some(Context::new(cpu_status, tracked_allocations))),
            is_dead: AtomicBool::new(false),
            is_removed: AtomicBool::new(false),
            parent_process: parent_process.clone(),
            scheduler: UnsafeCell::new(None),
            next: UnsafeCell::new(None),
            prev: UnsafeCell::new(None),
        }
    }

    /// Returns a mutable reference to the next thread in the scheduler's queue.
    /// # Safety
    /// the caller must take a lock on the scheduler before modifying this.
    pub unsafe fn next_mut(&self) -> &mut Option<ArcThread> {
        unsafe { &mut *self.next.get() }
    }

    /// Returns a mutable reference to the previous thread in the scheduler's queue.
    /// # Safety
    /// the caller must take a lock on the scheduler before modifying this.
    pub unsafe fn prev_mut(&self) -> &mut Option<ArcThread> {
        unsafe { &mut *self.prev.get() }
    }

    /// Returns a reference to the next thread in the scheduler's queue.
    /// # Safety
    /// The caller must be the scheduler.
    pub unsafe fn next(&self) -> &Option<ArcThread> {
        unsafe { self.next_mut() }
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

    /// Cleans up the thread's Context
    /// will finish cleanup when the context is dropped
    ///
    /// # Safety
    /// This function is unsafe because it can be called from any thread, and it will
    /// modify the thread's Context. It is the caller's responsibility to ensure that
    /// the thread is not currently running.
    pub unsafe fn cleanup(&self) {
        let context =
            unsafe { (&mut *self.context.get()).take() }.expect("Thread was already removed");
        drop(context);

        self.is_removed
            .store(true, core::sync::atomic::Ordering::Release);
    }

    pub fn status_mut<'a>(&'a self) -> SpinLockGuard<'a, ContextStatus> {
        self.status.lock()
    }

    /// Blocks the current thread forever, making sure it is not running first
    pub fn block_forever(&self) {
        loop {
            let mut status = self.status.lock();
            if status.is_running() {
                drop(status);
                current::yield_now();
            } else {
                *status = ContextStatus::Blocked(BlockedReason::BlockedForever);
                break;
            }
        }
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
    pub fn wait_for_thread(&self, thread: ArcThread) {
        self.set_status(ContextStatus::Blocked(BlockedReason::WaitingForThread(
            thread,
        )));
    }

    /// Should only be called by the current thread
    pub fn wait_for_futex(&self, addr: *const AtomicU32, with_value: u32, timeout_ms: u64) -> u128 {
        let timeout_at = time!(ms) as u128 + timeout_ms as u128;
        self.set_status(ContextStatus::Blocked(BlockedReason::WaitOnFutex {
            addr,
            value: with_value,
            timeout_wake_at: timeout_at,
        }));

        timeout_at
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
pub fn current() -> ArcThread {
    Scheduler::get().current_thread()
}

/// Returns the current process ID, that is the ID of the process executing this code right now.
///
/// faster than [`current()`]`.process().pid()`
pub fn current_pid() -> Pid {
    Scheduler::get().current_thread_ref().process().pid()
}
