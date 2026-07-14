//! Defines the current smallest unit of execution in the scheduler (a Task) that is a thread.

use core::{
    cell::UnsafeCell,
    fmt::Debug,
    hash::{Hash, Hasher},
    mem::ManuallyDrop,
    num::NonZero,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    arch::{threading::CPUStatus, without_interrupts},
    debug,
    process::{Pid, Process, mem::TrackedMemoryAllocation},
    scheduler::{SCHEDULER, SchedulePriority, Scheduler, ThreadScheduleReason},
    thread,
    timer::time_since_boot_ms,
    utils::locks::{SPIN_AMOUNT, SpinLock, SpinLockGuard},
};

pub mod current;

/// Thread ID, a unique identifier for a thread.
pub type Tid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ContextPriority {
    Low,
    Medium,
    High,
    Immediate,
}

impl ContextPriority {}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedReason {
    // /// The thread is sleeping until [`.0`] ms of boot time is reached
    // SleepingUntil(u64),
    // SleepingFor(NonZero<u64>),
    Waiting,
    Sleeping,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextStatus {
    /// Thread is currently running.
    Running,
    /// Thread is ready to run, but not currently running.
    Runnable,
    /// Thread is already blocked, and not running.
    Blocked(BlockedReason),
    /// Thread is being blocked, and may be running.
    Blocking(BlockedReason),
}

use alloc::sync::Arc;
use safa_abi::process::RawContextPriority;

#[derive(Debug)]
struct Slot {
    thread: ArcThread,
    next_slot_ptr: NonNull<Option<ArcThread>>,
}

pub struct ThreadList {
    head: Option<Slot>,
    len: usize,
}

impl Debug for ThreadList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut de = f.debug_struct("ThreadList");
        de.field_with("threads", |f| {
            let mut list = f.debug_list();

            if let Some(ref head) = self.head {
                let mut current = Some(&head.thread);
                while let Some(thread) = current {
                    list.entry(&Arc::as_ptr(&thread));
                    current = unsafe { thread.next.as_ref_unchecked().as_ref() };
                }
            }

            list.finish()
        })
        .finish()
    }
}

impl ThreadList {
    /// Create an empty thread list
    pub const fn new_empty() -> Self {
        Self { head: None, len: 0 }
    }

    /// Safety:
    /// Thread must not be a part of another list
    pub unsafe fn new_from_thread(thread: ArcThread) -> Self {
        let next_ref = unsafe { thread.next.as_ref_unchecked() };

        let next_slot_ptr = NonNull::from_ref(next_ref);
        Self {
            head: Some(Slot {
                thread,
                next_slot_ptr,
            }),
            len: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the list is empty
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Push a thread to the back of the list
    /// # Safety:
    /// Thread must not be a part of another list
    pub unsafe fn push_back(&mut self, thread: ArcThread) {
        match self.head {
            None => {
                *self = unsafe { ThreadList::new_from_thread(thread) };
            }
            Some(Slot {
                thread: _,
                next_slot_ptr: ref mut tail_ptr,
            }) => {
                let next_ptr = thread.next.get();
                unsafe { *tail_ptr.as_mut() = Some(thread) };
                *tail_ptr = unsafe { NonNull::new_unchecked(next_ptr) };
                self.len += 1;
            }
        }
    }

    /// Push a thread to the front of the list
    /// # Safety:
    /// Thread must not be a part of another list
    pub unsafe fn push_front(&mut self, new_head: ArcThread) {
        match self.head.take() {
            None => {
                *self = unsafe { ThreadList::new_from_thread(new_head) };
            }
            Some(Slot {
                thread: old_head,
                next_slot_ptr,
            }) => {
                unsafe { *new_head.next.get() = Some(old_head) };
                self.head = Some(Slot {
                    thread: new_head,
                    next_slot_ptr,
                });
                self.len += 1;
            }
        }
    }

    /// Pop the front thread from the list
    pub fn pop_front(&mut self) -> Option<ArcThread> {
        match core::mem::replace(&mut self.head, None) {
            None => None,
            Some(Slot {
                thread: head,
                next_slot_ptr,
            }) => {
                let head_next_ptr = head.next.get();
                let head_next = unsafe { &mut *head_next_ptr };

                if let Some(next) = head_next.take() {
                    let next_slot_ptr = if core::ptr::eq(next_slot_ptr.as_ptr(), head_next_ptr) {
                        unsafe { NonNull::new_unchecked(next.next.get()) }
                    } else {
                        next_slot_ptr
                    };

                    let slot = Slot {
                        thread: next,
                        next_slot_ptr,
                    };

                    self.head = Some(slot);
                }

                self.len -= 1;
                Some(head)
            }
        }
    }

    /// Appends other thread list to the end of this thread list leaving the other list empty
    pub fn append(&mut self, other: &mut Self) {
        if let Some(other_head) = other.head.take() {
            let head_thread = other_head.thread;
            unsafe { self.push_back(head_thread) };
            self.head
                .as_mut()
                .expect("push back should make head some")
                .next_slot_ptr = other_head.next_slot_ptr;
            self.len += other.len - 1;
            other.len = 0;
        }
    }
}

/// A shared reference to a Thread, provides extra safety checks and methods over an Arc<Thread>
#[derive(Debug, Clone)]
pub struct ArcThread(Arc<Thread>);

impl PartialEq for ArcThread {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ArcThread {}

impl Hash for ArcThread {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

unsafe impl Send for ArcThread {}

impl ArcThread {
    pub fn new(thread: Thread) -> Self {
        Self(Arc::new(thread))
    }
    /// Remove this thread from the thread list
    /// # Safety
    /// If the thread is the current thread, this function must be called without interrupts on
    unsafe fn remove_self(&self) {
        let is_current = thread::is_current(self);

        if !is_current {
            self.block_dead();
        } else {
            self.should_terminate.store(true, Ordering::Relaxed);
            self.set_status(ContextStatus::Blocking(BlockedReason::Dead));
        }

        // TODO: cleanup requires lock on threads-manager
        // if !is_current {
        //     // the thread isn't running we can drop it now
        //     unsafe { self.cleanup() };
        // };

        // scheduler.sub_thread_count();
    }

    #[must_use]
    /// Kills the thread without removing it from the process list,
    /// remove the thread from the Scheduler's task list
    /// # Safety
    /// The caller must remove the thread from the parent process's thread list.
    /// If this was called from the current thread, the caller must run it without interrupts.
    /// If this was the last thread in the process, the process must be killed by the caller.
    ///
    /// Returns whether the thread was successfully killed by the caller.
    pub unsafe fn soft_kill(&self, process_dead: bool) -> bool {
        if self
            .is_dying
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let is_current = thread::is_current(self);

            if is_current {
                // another thread is killing this thread
                self.set_status(ContextStatus::Blocking(BlockedReason::Dead));
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
    pub unsafe fn kill(&self, exit_code: isize) {
        let process = &self.parent_process;
        unsafe { Process::on_thread_exit(process, self, exit_code) };
    }

    #[must_use = "returns if it slept or not"]
    /// Prepares the current thread to sleep for a given number of milliseconds.
    ///
    /// Sleep will begin on the next call to [`thread::current::yield_now()`].
    /// # Safety
    /// The caller must handle the case that this is the current thread carefully and interrupts must be disabled.
    pub unsafe fn prepare_sleep_for_ms(&self, ms: NonZero<u64>) -> bool {
        if self.should_terminate() {
            return false;
        }

        let current_time_ms = time_since_boot_ms();
        let time = ms.saturating_add(current_time_ms);

        unsafe {
            self.scheduler()
                .schedule_thread(self.clone(), ThreadScheduleReason::SleepUntil(time));
        }
        let mut status_mut = self.status_mut();
        unsafe { *self.timeouted.get() = false };
        *status_mut = ContextStatus::Blocking(BlockedReason::Sleeping);
        true
    }

    #[inline]
    fn loop_until_blocked_inner<'a>(
        &'a self,
        can_yield: bool,
    ) -> Result<(BlockedReason, SpinLockGuard<'a, ContextStatus>), ()> {
        loop {
            core::hint::spin_loop();
            let status = self.status.lock();
            // Wait for the thread to fully block before unblocking it.
            match &*status {
                ContextStatus::Blocked(r) => break Ok((*r, status)),
                ContextStatus::Blocking(_) => {}
                _ => break Err(()),
            }

            drop(status);
            if can_yield {
                thread::current::yield_now();
            } else {
                for _ in 0..(SPIN_AMOUNT / 10) {
                    core::hint::spin_loop();
                }
            }
        }
    }

    #[inline(always)]
    fn loop_until_blocked(&self, can_yield: bool) -> Result<BlockedReason, ()> {
        self.loop_until_blocked_inner(can_yield).map(|(o, _)| o)
    }

    #[inline(always)]
    /// Loops until the thread is blocked with the given reason `expected`, then replaces it's status with `new`.
    ///
    /// If the thread blocks with the expected reason, it's status is replaced with `new` and Ok(status_guard) is returned.
    /// If the thread blocks with a different reason, Err(Some(reason)) is returned.
    /// If the thread is not blocked, Err(None) is returned.
    fn loop_until_blocked_compare_exchange<'a>(
        &'a self,
        expected: BlockedReason,
        new: ContextStatus,
        can_yield: bool,
    ) -> Result<SpinLockGuard<'a, ContextStatus>, Option<BlockedReason>> {
        self.loop_until_blocked_inner(can_yield)
            .map(|(o, mut status)| {
                if *status == ContextStatus::Blocked(expected) {
                    *status = new;
                    Ok(status)
                } else {
                    Err(Some(o))
                }
            })
            .map_err(|()| None)
            .flatten()
    }
    /// Wakes up a blocked thread, whatever the block reason is.
    /// # Arguments
    /// * `timeouted` - Whether the thread was woken up due to a timeout.
    pub fn wake_up(&self, timeouted: bool) {
        let wokeup = without_interrupts(|| {
            let mut status = self.status.lock();
            match &*status {
                ContextStatus::Blocked(r) => {
                    let r = *r;
                    match r {
                        BlockedReason::Dead => {}
                        BlockedReason::Waiting | BlockedReason::Sleeping => {
                            *status = ContextStatus::Runnable;
                        }
                    }

                    Some(r)
                }
                ContextStatus::Blocking(_) => {
                    drop(status);
                    Some(
                        self.loop_until_blocked(true)
                            .expect("Blocking thread should go only from blocking to blocked"),
                    )
                }

                _ => None,
            }
        });

        if let Some(reason) = wokeup {
            let schedule_reason = match reason {
                BlockedReason::Dead => return,
                BlockedReason::Sleeping => {
                    unsafe { *self.timeouted.get() = timeouted }
                    ThreadScheduleReason::UnblockTimeoutOperation
                }
                BlockedReason::Waiting => ThreadScheduleReason::Unblocked,
            };

            unsafe {
                self.scheduler()
                    .schedule_thread(self.clone(), schedule_reason)
            };
        }
    }

    /// Called before a thread is woken up from a sleep operation.
    /// # Safety
    /// Designed to only be called by the scheduler.
    pub unsafe fn before_sleep_wakeup(&self) {
        if let Ok(status_guard) = self.loop_until_blocked_compare_exchange(
            BlockedReason::Sleeping,
            ContextStatus::Runnable,
            false,
        ) {
            unsafe { *self.timeouted.get() = true }
            drop(status_guard);
        }
    }

    /// Blocks the current thread forever, making sure it is not running first
    fn block_dead(&self) {
        crate::debug!(Thread, "blocking: {}", self.tid());
        let status = self.status.lock();
        self.should_terminate.store(true, Ordering::Relaxed);
        match *status {
            ContextStatus::Blocked(BlockedReason::Dead) => {}
            ContextStatus::Blocked(_) | ContextStatus::Blocking(_) => {
                drop(status);
                self.wake_up(true);
            }
            _ => {}
        }

        crate::debug!(Thread, "blocked: {}", self.tid());
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

pub struct Thread {
    id: Tid,
    pub schedule_priority: UnsafeCell<SchedulePriority>,
    priority: ContextPriority,
    status: SpinLock<ContextStatus>,
    context: UnsafeCell<Context>,

    is_dying: AtomicBool,
    pub timeouted: UnsafeCell<bool>,
    should_terminate: AtomicBool,
    is_dead: AtomicBool,
    parent_process: Arc<Process>,
    /// Kernel stack memory mapping, so that it can be freed when the thread is killed.
    kernel_stack: UnsafeCell<ManuallyDrop<TrackedMemoryAllocation>>,
    /// User stack and TLS memory mapping, so that it can be freed when the thread is killed.
    thread_mem: UnsafeCell<ManuallyDrop<TrackedMemoryAllocation>>,
    thread_tls: UnsafeCell<ManuallyDrop<Option<TrackedMemoryAllocation>>>,

    /// The scheduler that this thread belongs to.
    /// null until scheduled
    pub scheduler: SpinLock<Option<NonNull<Scheduler>>>,
    // For safety we have to follow 2 rules:
    // 1. reads must be performed by the scheduler
    // 2. writes must be performed with the scheduler's lock held
    next: UnsafeCell<Option<ArcThread>>,
}

impl Debug for Thread {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe {
            f.debug_struct("Thread")
                .field("tid", &self.id)
                .field("parent_pid", &self.parent_process.pid())
                .field("parent_ppid", &self.parent_process.ppid())
                .field("schedule_policy", &*self.schedule_priority.get())
                .field("status", &self.status.try_lock().as_ref().map(|v| &**v))
                .field("kstack", &*self.kernel_stack.get())
                .field("other_mem", &*self.thread_mem.get())
                .field("tls", &*self.thread_tls.get())
                .field("has_next", &(*self.next.get()).is_some())
                .finish()
        }
    }
}

impl Thread {
    /// Sets the parent scheduler of this thread.
    ///
    /// Doesn't acquire any locks so it is unsafe.
    pub unsafe fn set_scheduler(&self, schd: &'static Scheduler) {
        unsafe { *self.scheduler.get() = Some(NonNull::from_ref(schd)) }
    }

    /// Tries to set the parent scheduler of this thread, returning `true` if successful.
    ///
    /// Returns `false` if the scheduler lock is currently held by another thread.
    pub fn try_set_scheduler(&self, schd: &'static Scheduler) -> bool {
        if let Some(mut guard) = self.scheduler.try_lock() {
            *guard = Some(NonNull::from_ref(schd));
            true
        } else {
            false
        }
    }

    pub fn new(
        cid: Tid,
        cpu_status: CPUStatus,
        parent_process: &Arc<Process>,
        priority: ContextPriority,
        kernel_stack: TrackedMemoryAllocation,
        thread_mem: TrackedMemoryAllocation,
        thread_tls: Option<TrackedMemoryAllocation>,
    ) -> Self {
        Self {
            schedule_priority: UnsafeCell::new(SchedulePriority::new()),
            timeouted: UnsafeCell::new(false),
            id: cid,
            priority,
            status: SpinLock::new(ContextStatus::Runnable),
            context: UnsafeCell::new(Context::new(cpu_status)),
            is_dying: AtomicBool::new(false),
            is_dead: AtomicBool::new(false),
            parent_process: parent_process.clone(),
            scheduler: SpinLock::new(None),
            next: UnsafeCell::new(None),
            should_terminate: AtomicBool::new(false),
            kernel_stack: UnsafeCell::new(ManuallyDrop::new(kernel_stack)),
            thread_tls: UnsafeCell::new(ManuallyDrop::new(thread_tls)),
            thread_mem: UnsafeCell::new(ManuallyDrop::new(thread_mem)),
        }
    }

    /// Returns true if the thread should terminate
    pub fn should_terminate(&self) -> bool {
        self.should_terminate.load(Ordering::Relaxed)
    }

    /// Returns true if the thread operation timed out
    /// # Safety
    /// The caller must be from the given thread.
    pub unsafe fn operation_timeout(&self) -> bool {
        unsafe { core::mem::take(&mut *self.timeouted.get()) }
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
        self.is_dead.load(core::sync::atomic::Ordering::Relaxed)
    }

    #[must_use = "Returns true if the thread was successfully cleaned up"]
    /// Cleans up the thread's Context
    /// will finish cleanup when the context is dropped
    ///
    /// # Safety
    /// This function is unsafe because it can be called from any thread, and it will
    /// modify the thread's Context. It is the caller's responsibility to ensure that
    /// the thread is not currently running, and that the thread wasn't already cleaned up.
    pub unsafe fn try_cleanup(&self) -> bool {
        let mut manager = self.parent_process.threads_manager();
        unsafe { ManuallyDrop::drop(&mut *self.kernel_stack.get()) };
        unsafe { ManuallyDrop::drop(&mut *self.thread_mem.get()) };
        unsafe { ManuallyDrop::drop(&mut *self.thread_tls.get()) };
        manager.remove(self.tid());
        true
    }

    pub fn status_mut<'a>(&'a self) -> SpinLockGuard<'a, ContextStatus> {
        self.status.lock()
    }

    /// Blocks the current thread temporarily without a condition to wake up at, doesn't begin sleeping until the next thread yield.
    /// Returns `true` if the thread was blocked, `false` if it should immediately continue as if it was interrupted.
    ///
    /// # Safety
    /// Safe to call, but may cause a deadlock if not used correctly,
    /// make sure to disable interrupts before calling this and drop all the local locks after calling this, then thread yield.
    pub unsafe fn block_waiting(&self) -> bool {
        let mut status = self.status.lock();
        if !self.should_terminate() {
            *status = ContextStatus::Blocking(BlockedReason::Waiting);
            true
        } else {
            false
        }
    }

    /// Safety: Has to be the current thread and interrupts must be disabled.
    pub unsafe fn scheduler(&self) -> &'static Scheduler {
        unsafe {
            (*self.scheduler.get())
                .expect("Scheduler should never be null")
                .as_ref()
        }
    }

    /// Should only be called by the current thread or the scheduler or on a sleeping thread
    pub fn set_status(&self, status: ContextStatus) {
        let mut guard = self.status.lock();
        if status == ContextStatus::Running
            && *guard == ContextStatus::Blocking(BlockedReason::Dead)
        {
            return;
        }

        debug_assert!(
            *guard == ContextStatus::Runnable
                || *guard == ContextStatus::Running
                || (matches!((&*guard, &status), (ContextStatus::Blocking(x), ContextStatus::Blocked(y)) if x == y)
                    || (matches!(
                        *guard,
                        ContextStatus::Blocked(BlockedReason::Waiting | BlockedReason::Sleeping)
                    )))
                || (matches!(status, ContextStatus::Blocking(BlockedReason::Dead))
                    && status == *guard),
            "Cannot switch status from {:?} to {:?}, thread id: {}:{}",
            *guard,
            status,
            self.process().pid(),
            self.id
        );
        *guard = status;
    }
}

#[derive(Debug)]
pub struct Context {
    cpu_status: CPUStatus,
}

impl Context {
    #[inline(always)]
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
///
/// May be faster than [`with_current_arc`] as it doesn't do Arc cloning.
/// fastest version is [`with_current_unsafe`].
pub fn with_current_ref<F, R>(f: F) -> R
where
    F: FnOnce(&Thread) -> R,
{
    // Safety:
    // converts the ArcThread into a reference to the Thread struct, with no interrupts so the scheduler won't change.
    f(without_interrupts(|| unsafe {
        SCHEDULER.current_thread_ref()
    }))
}

/// Safety: Must be executed with interrupts disabled, and no thread switches shall occur while accessing the given pointer.
///
/// Theoretically faster than [`with_current_ref`] and [`with_current_arc`].
pub unsafe fn with_current_unsafe<F, R>(f: F) -> R
where
    F: FnOnce(*const ArcThread) -> R,
{
    unsafe { f(SCHEDULER.current_thread_ref()) }
}

/// Returns true if [`other`] is the current thread.
pub fn is_current(other: &ArcThread) -> bool {
    unsafe { without_interrupts(|| with_current_unsafe(|curr| Arc::ptr_eq(&**curr, other))) }
}

/// Returns the current process ID, that is the ID of the process executing this code right now.
pub fn current_pid() -> Pid {
    with_current_ref(|curr| curr.process().pid())
}
