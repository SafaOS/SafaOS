//! Defines the current smallest unit of execution in the scheduler (a Task) that is a thread.

use core::{
    cell::UnsafeCell,
    fmt::Debug,
    num::NonZero,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    arch::{threading::CPUStatus, without_interrupts},
    debug, eve,
    process::{Pid, Process, resources::Ri},
    scheduler::Scheduler,
    thread, time,
    utils::locks::{Mutex, SpinLock, SpinLockGuard},
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
            Self::High => 4,
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
    SleepingUntil(u64),
    Waiting,
    Dead,
}

impl BlockedReason {
    pub fn block_lifted(&self) -> bool {
        match self {
            Self::SleepingUntil(n) => time!(ms) >= *n,
            Self::Waiting => false,
            Self::Dead => false,
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
}

use alloc::{sync::Arc, vec::Vec};
use safa_abi::process::RawContextPriority;

/// A shared reference to a Thread, provides extra safety checks and methods over an Arc<Thread>
#[derive(Debug, Clone)]
pub struct ArcThread(Arc<Thread>);
unsafe impl Send for ArcThread {}

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

        let is_current = thread::is_current(self);
        if !is_current {
            self.block_dead();
        }

        unsafe {
            eve::schedule_thread_cleanup(self.clone(), scheduler.context_switches_count_ref())
        };
        if is_current {
            self.set_status(ContextStatus::Blocked(BlockedReason::Dead));
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

        // TODO: cleanup requires lock on threads-manager
        // if !is_current {
        //     // the thread isn't running we can drop it now
        //     unsafe { self.cleanup() };
        // };

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

    #[must_use]
    /// Kills the thread without removing it from the process list,
    /// remove the thread from the Scheduler's task list
    /// # Safety
    /// The caller must remove the thread from the parent process's thread list.
    /// If this was called from the current thread, the caller must run it without interrupts.
    /// If this was the last thread in the process, the process must be killed by the caller.
    pub unsafe fn soft_kill(&self, process_dead: bool) -> bool {
        if self
            .is_dying
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let is_current = thread::is_current(self);

            if is_current {
                // another thread is killing this thread
                self.set_status(ContextStatus::Blocked(BlockedReason::Dead));
                return false;
            } else {
                while !self.is_dead() {
                    core::hint::spin_loop();
                }
                return false;
            }
        }

        unsafe {
            self.remove_self();
        }

        self.is_dead.store(true, Ordering::SeqCst);
        debug!(
            Process,
            "Thread {}:{} ({}) THREAD EXITED, process dead: {process_dead}",
            self.process().pid(),
            self.tid(),
            self.process().name(),
        );
        true
    }

    /// Kills the thread removing it from the parent process's thread list unlike [`soft_kill`],
    /// also handles killing the process if it was the last thread and running without interrupts.
    ///
    /// # Safety
    /// The caller must handle the case that this is the current thread carefully, interrupts must be disabled and all caller resources shall be dropped.
    pub unsafe fn kill(&self, exit_code: usize) {
        let process = &self.parent_process;
        unsafe { Process::on_thread_exit(process, self, exit_code) };
    }

    // Puts this thread to sleep in the given wait queue, for the given reason [`reason`],
    // doesn't immediately begin sleeping until the next thread yield.
    //
    // # Safety
    // This function is safe but if used incorrectly can lead to deadlocks, please run without interrupts and then drop any local locks after calling this function, before yielding to begin the sleep.
    // pub fn sleep_in_queue<const AVERAGE: usize, Reason>(
    //     self,
    //     queue: &mut WaitQueue<AVERAGE, Reason>,
    //     reason: Reason,
    // ) {
    //     queue.push(self, reason);
    // }

    // pub fn sleep_in_queue_with_timeout<const AVERAGE: usize, Reason>(
    //     self,
    //     queue: &mut WaitQueueWithTimeout<AVERAGE, Reason>,
    //     reason: Reason,
    //     duration: Option<NonZero<u64>>,
    // ) -> Option<NonZero<u64>> {
    //     queue.push(self, reason, duration)
    // }
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
    context: UnsafeCell<Context>,

    is_dying: AtomicBool,
    should_terminate: UnsafeCell<bool>,
    is_dead: AtomicBool,
    parent_process: Arc<Process>,
    owned_resources: Mutex<Vec<Ri>>,

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
    /// Takes ownership of a given resource list
    pub fn take_resources(&self, ri: &[Ri]) {
        let mut owned_resources = self.owned_resources.lock();
        owned_resources.extend_from_slice(ri);
    }

    pub fn new(
        cid: Tid,
        cpu_status: CPUStatus,
        parent_process: &Arc<Process>,
        priority: ContextPriority,
    ) -> Self {
        Self {
            owned_resources: Mutex::new(Vec::new()),
            id: cid,
            priority,
            status: SpinLock::new(ContextStatus::Runnable),
            context: UnsafeCell::new(Context::new(cpu_status)),
            is_dying: AtomicBool::new(false),
            is_dead: AtomicBool::new(false),
            parent_process: parent_process.clone(),
            scheduler: UnsafeCell::new(None),
            next: UnsafeCell::new(None),
            prev: UnsafeCell::new(None),
            should_terminate: UnsafeCell::new(false),
        }
    }

    /// Returns true if the thread should terminate
    pub fn should_terminate(&self) -> bool {
        unsafe { *self.should_terminate.get() }
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

    pub const unsafe fn context_unchecked(&self) -> &mut Context {
        unsafe { &mut *self.context.get() }
    }

    pub const fn tid(&self) -> Tid {
        self.id
    }

    pub fn is_dead(&self) -> bool {
        self.is_dead.load(core::sync::atomic::Ordering::SeqCst)
    }

    /// Cleans up the thread's Context
    /// will finish cleanup when the context is dropped
    ///
    /// # Safety
    /// This function is unsafe because it can be called from any thread, and it will
    /// modify the thread's Context. It is the caller's responsibility to ensure that
    /// the thread is not currently running.
    pub unsafe fn cleanup(&self) {
        {
            let mut resource_manager = self.parent_process.resources_mut();
            let owned_resources = self.owned_resources.try_lock().expect("Thread is active");

            for resource in &*owned_resources {
                resource_manager.remove_resource(*resource);
            }
        }

        self.parent_process.threads_manager().remove(self.tid());
    }

    pub fn status_mut<'a>(&'a self) -> SpinLockGuard<'a, ContextStatus> {
        self.status.lock()
    }

    /// Blocks the current thread forever, making sure it is not running first
    pub fn block_dead(&self) {
        // Safety:
        // - Only block_dead muttates this
        // - Its only goes from false to true and not backwards, the time threads read this after it is true doesn't matter.
        unsafe { *self.should_terminate.get() = true };
        loop {
            let mut status = self.status.lock();

            match *status {
                ContextStatus::Runnable
                    // Safety: we hold a lock on status, and context shall not be accessed before holding a lock on status, perhaps this can be expressed better?
                    if unsafe { self.context_unchecked().cpu_status.at().is_in_lower_half() } =>
                {
                    *status = ContextStatus::Blocked(BlockedReason::Dead);
                    break;
                }
                ContextStatus::Running | ContextStatus::Runnable => {}
                ContextStatus::Blocked(BlockedReason::Dead) => {
                    break;
                }
                ContextStatus::Blocked(
                    BlockedReason::SleepingUntil(_) | BlockedReason::Waiting,
                ) => {
                    *status = ContextStatus::Runnable;
                }
            }

            drop(status);
            current::yield_now()
        }
    }

    /// Blocks the current thread temporarily without a condition to wake up at, doesn't begin sleeping until the next thread yield.
    /// # Safety
    /// Safe to call, but may cause a deadlock if not used correctly,
    /// make sure to disable interrupts before calling this and drop all the local locks after calling this, then thread yield.
    pub fn block_waiting(&self) {
        without_interrupts(|| {
            let mut status = self.status.lock();
            *status = ContextStatus::Blocked(BlockedReason::Waiting)
        });
    }

    /// Wakes up a blocked thread, whatever the block reason is.
    pub fn wake_up(&self) {
        without_interrupts(|| {
            let mut status = self.status.lock();
            if matches!(*status, ContextStatus::Blocked(_)) {
                *status = ContextStatus::Runnable;
            }
        });
    }

    /// Should only be called by the current thread or the scheduler or on a sleeping thread
    pub fn set_status(&self, status: ContextStatus) {
        *self.status.lock() = status;
    }

    /// Should only be called by the current thread
    pub fn sleep_for_ms(&self, ms: NonZero<u64>) -> NonZero<u64> {
        let mut status_mut = self.status_mut();
        let timeout_at =
            unsafe { NonZero::new_unchecked(time!(ms) as u64) }.saturating_add(ms.get());
        *status_mut = ContextStatus::Blocked(BlockedReason::SleepingUntil(timeout_at.get()));
        timeout_at
    }
}

#[derive(Debug)]
pub struct Context {
    cpu_status: CPUStatus,
}

impl Context {
    pub const fn set_cpu_status(&mut self, status: CPUStatus) {
        self.cpu_status = status;
    }

    pub unsafe fn cpu_status(&mut self) -> core::ptr::NonNull<CPUStatus> {
        unsafe { core::ptr::NonNull::new_unchecked(&mut self.cpu_status) }
    }

    pub(super) fn new(cpu_status: CPUStatus) -> Self {
        Context { cpu_status }
    }
}

/// Executes [`f`] on the current thread returning the results.
pub fn with_current<F, R>(f: F) -> R
where
    F: FnOnce(&ArcThread) -> R,
{
    // Safety:
    // The reference would always point to the current thread as long as it's really is the current thread,
    // and The lifetime of this reference is local to this function which is running within this thread.
    f(unsafe { Scheduler::get().current_thread_ref() })
}

/// Returns true if [`other`] is the current thread.
pub fn is_current(other: &ArcThread) -> bool {
    with_current(|curr| Arc::ptr_eq(curr, other))
}

/// Returns the current process ID, that is the ID of the process executing this code right now.
pub fn current_pid() -> Pid {
    with_current(|curr| curr.process().pid())
}
