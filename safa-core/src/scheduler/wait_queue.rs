// Hack to allow timeouts as an option
#![allow(private_bounds)]

use core::num::NonZero;
use core::ops::{Deref, DerefMut};

use alloc::sync::Arc;
use safa_abi::errors::IntoErr;
use smallvec::SmallVec;
use thiserror::Error;

use crate::arch::without_interrupts;
use crate::thread;
use crate::thread::ArcThread;
use crate::utils::locks::{Mutex, MutexGuard, SpinLock, SpinLockGuard};

const MIN_WAIT_THREADS: usize = 4;

pub trait GuardedWaitQueue<'a, const AVERAGE: usize, Reason> {
    type Guard: Deref<Target = WaitQueue<AVERAGE, Reason>> + DerefMut + 'a;

    fn no_interrupts() -> bool;
    fn acquire_lock(&'a self) -> Self::Guard;
    fn drop_lock(guard: Self::Guard);
}

impl<'a, const AVERAGE: usize, Reason: 'a> GuardedWaitQueue<'a, AVERAGE, Reason>
    for Mutex<WaitQueue<AVERAGE, Reason>>
{
    type Guard = MutexGuard<'a, WaitQueue<AVERAGE, Reason>>;

    fn no_interrupts() -> bool {
        false
    }

    fn acquire_lock(&'a self) -> Self::Guard {
        self.lock()
    }

    fn drop_lock(guard: Self::Guard) {
        drop(guard);
    }
}

impl<'a, const AVERAGE: usize, Reason: 'a> GuardedWaitQueue<'a, AVERAGE, Reason>
    for SpinLock<WaitQueue<AVERAGE, Reason>>
{
    type Guard = SpinLockGuard<'a, WaitQueue<AVERAGE, Reason>>;

    fn no_interrupts() -> bool {
        true
    }

    fn acquire_lock(&'a self) -> Self::Guard {
        self.lock()
    }

    fn drop_lock(guard: Self::Guard) {
        drop(guard);
    }
}

/// Represents a lock on a [`WaitQueue`] thats held before beginning a wait operation,
/// call enter_wait on this to sleep the current thread in the queue,
///
/// This lock helps guarantee nobody will wake threads before this thread is sleeping for a given condition,
/// Ensure to make sure the condition applies after holding this lock, before actually beginning sleeping otherwise dropping this is perfectly safe.
#[derive(Debug)]
pub struct PendingWait<'a, const AVERAGE: usize, Reason, L: GuardedWaitQueue<'a, AVERAGE, Reason>>(
    L::Guard,
    &'a L,
);

impl<'a, const AVERAGE: usize, Reason, L: GuardedWaitQueue<'a, AVERAGE, Reason>> Deref
    for PendingWait<'a, AVERAGE, Reason, L>
{
    type Target = L::Guard;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const AVERAGE: usize, Reason> Mutex<WaitQueue<AVERAGE, Reason>> {
    /// Returns a [`PendingWait`] instance that holds a lock on self, afterwards call [`PendingWait::enter_wait`].
    pub fn prepare_wait<'s>(&'s self) -> PendingWait<'s, AVERAGE, Reason, Self> {
        PendingWait(self.lock(), self)
    }
}

impl<const AVERAGE: usize, Reason> SpinLock<WaitQueue<AVERAGE, Reason>> {
    /// Returns a [`PendingWait`] instance that holds a lock on self, afterwards call [`PendingWait::enter_wait`].
    ///
    /// Safety: The caller must ensure interrupts are disabled.
    pub unsafe fn prepare_wait<'s>(&'s self) -> PendingWait<'s, AVERAGE, Reason, Self> {
        PendingWait(self.lock(), self)
    }
}

impl<'a, const AVERAGE: usize, Reason, L: GuardedWaitQueue<'a, AVERAGE, Reason>>
    PendingWait<'a, AVERAGE, Reason, L>
{
    /// Applies the [`PendingWait`], causing the current thread to sleep for at most `timeout_after` ms if Some, until a wake operation on the [`WaitQueue`] happens.
    pub fn enter_wait(
        self,
        reason: Reason,
        timeout: Option<NonZero<u64>>,
    ) -> Result<(), WaitError> {
        let begin_sleep = |thread: &ArcThread, guard: L::Guard| {
            let not_done = if let Some(timeout) = timeout {
                // Returns true if we should yield
                unsafe { thread.prepare_sleep_for_ms(timeout) }
            } else {
                unsafe { thread.block_waiting() };
                // Not done yet
                true
            };

            L::drop_lock(guard);
            if not_done {
                thread::current::yield_now();
            }
        };

        let f = |thread: &ArcThread| {
            if thread.should_terminate() {
                return Err(WaitError::ForceTerminated);
            }

            let (mut wait_queue_guard, wait_queue) = (self.0, self.1);

            if L::no_interrupts() {
                // Ensure we won't thread yield
                // because if we do, a dead-lock may occur because of the allocator.
                // FIXME: SpinLock the allocator?

                assert!(
                    wait_queue_guard.threads.capacity() >= 1,
                    "WaitQueue cannot hold one more thread"
                );
            }

            wait_queue_guard.threads.push((thread.clone(), reason));
            begin_sleep(thread, wait_queue_guard);

            let remove_self = || {
                let mut wait_queue = wait_queue.acquire_lock();
                let index = wait_queue
                    .threads
                    .iter()
                    .position(|(t, _)| Arc::ptr_eq(t, &thread));

                if let Some(i) = index {
                    // TODO: should we swap_remove?
                    wait_queue.threads.swap_remove(i);
                }
            };

            if thread.should_terminate() {
                remove_self();
                return Err(WaitError::ForceTerminated);
            }

            if unsafe { thread.operation_timeout() } {
                remove_self();
                return Err(WaitError::Timeout);
            }

            Ok(())
        };

        if L::no_interrupts() {
            unsafe { thread::with_current_unsafe(|t| f(&*t)) }
        } else {
            without_interrupts(|| unsafe { thread::with_current_unsafe(|t| f(&(*t).clone())) })
        }
    }
}

/// An error during a Wait operation done on a [`PendingWait`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WaitError {
    /// Timeout reached
    #[error("Wait timeouted")]
    Timeout,
    /// Thread Terminated
    #[error("Thread terminated")]
    ForceTerminated,
}

impl IntoErr for WaitError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::ForceTerminated => safa_abi::errors::ErrorStatus::ForceTerminated,
            Self::Timeout => safa_abi::errors::ErrorStatus::Timeout,
        }
    }
}

/// A wait queue for waiting on threads given a condition,
/// and waking them up when the condition is met or when the queue is destroyed.
/// the order of the threads is not guaranteed.
///
/// [`AVERAGE`] is the average number of threads that will be stored in the wait queue, used to avoid heap allocations.
/// [`Reason`] is the reason type for waiting.
#[derive(Debug)]
pub struct WaitQueue<const AVERAGE: usize = MIN_WAIT_THREADS, Reason = ()> {
    threads: SmallVec<[(ArcThread, Reason); AVERAGE]>,
}

/// [`WaitQueue`] but timeouts are available
pub type WaitQueueWithTimeout<const AVERAGE: usize, Reason> = WaitQueue<AVERAGE, Reason>;

impl<const AVERAGE: usize, Reason> WaitQueue<AVERAGE, Reason> {
    /// Creates a new wait queue.
    pub const fn new() -> Self {
        Self {
            threads: SmallVec::new_const(),
        }
    }

    /// Wakes all threads in the wait queue.
    pub fn wake_all(&mut self) {
        for (thread, _) in self.threads.drain(..) {
            thread.wake_up(false);
        }
    }

    /// Wakes all threads in the wait queue that satisfy the given condition.
    pub fn wake_on_condition(&mut self, mut condition: impl FnMut(&mut Reason) -> bool) {
        self.threads.retain(|(thread, reason)| {
            if condition(reason) {
                thread.wake_up(false);
                false
            } else {
                true
            }
        });
    }

    /// Wakes at most `n` threads in the wait queue that satisfy the given condition.
    ///
    /// returns the number of threads that were woken up.
    pub fn wake_n_on_condition(
        &mut self,
        mut condition: impl FnMut(&mut Reason) -> bool,
        n: usize,
    ) -> usize {
        let mut count = 0;
        if count < n {
            self.threads.retain(|(thread, reason)| {
                if condition(reason) {
                    thread.wake_up(false);
                    count += 1;
                    false
                } else {
                    true
                }
            });
        }
        count
    }

    /// Attempts to pop a thread from the wait queue where the given function `d` returns Some(R),
    /// if any thread was successfully awaken returns the results of `d`.
    pub fn try_pop_one<R>(&mut self, d: impl Fn(&mut Reason) -> Option<R>) -> Option<R> {
        let mut results = None;
        self.wake_n_on_condition(
            |reason| {
                let attempt = d(reason);
                if attempt.is_some() {
                    results = attempt;
                    true
                } else {
                    false
                }
            },
            1,
        );
        results
    }

    /// Checks if the wait queue is empty.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    pub fn len(&self) -> usize {
        self.threads.len()
    }
}

impl<const AVERAGE: usize, Reason: Eq> WaitQueue<AVERAGE, Reason> {
    /// Wakes all threads in the wait queue that are waiting for the given reason.
    pub fn wake_equals(&mut self, reason: &Reason) {
        self.wake_on_condition(|r| r == reason)
    }
}

impl<const AVERAGE: usize, Reason> Drop for WaitQueue<AVERAGE, Reason> {
    fn drop(&mut self) {
        self.wake_all();
    }
}
