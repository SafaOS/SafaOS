// Hack to allow timeouts as an option
#![allow(private_bounds)]

use core::num::NonZero;

use alloc::sync::Arc;
use safa_abi::errors::IntoErr;
use smallvec::SmallVec;
use thiserror::Error;

use crate::arch::with_interrupts;
use crate::thread;
use crate::utils::locks::{Mutex, MutexGuard};
use crate::{thread::ArcThread, time};

const MIN_WAIT_THREADS: usize = 4;

/// Represents a lock on a [`WaitQueue`] thats held before beginning a wait operation,
/// call enter_wait on this to sleep the current thread in the queue,
///
/// This lock helps guarantee nobody will wake threads before this thread is sleeping for a given condition,
/// Ensure to make sure the condition applies after holding this lock, before actually beginning sleeping otherwise dropping this is perfectly safe.
#[derive(Debug)]
pub struct PendingWait<'a, const AVERAGE: usize, Reason, Timeout: TimeoutType>(
    MutexGuard<'a, WaitQueue<AVERAGE, Reason, Timeout>>,
    &'a Mutex<WaitQueue<AVERAGE, Reason, Timeout>>,
);

impl<const AVERAGE: usize, Reason, Timeout: TimeoutType>
    Mutex<WaitQueue<AVERAGE, Reason, Timeout>>
{
    /// Returns a [`PendingWait`] instance that holds a lock on self, afterwards call [`PendingWait::enter_wait`].
    pub fn prepare_wait<'s>(&'s self) -> PendingWait<'s, AVERAGE, Reason, Timeout> {
        PendingWait(self.lock(), self)
    }
}

impl<const AVERAGE: usize, Reason, Timeout: TimeoutType> PendingWait<'_, AVERAGE, Reason, Timeout> {
    fn enter_wait_inner(self, reason: Reason, timeout: Timeout) -> Result<(), WaitError> {
        thread::with_current(|thread| {
            if thread.should_terminate() {
                return Err(WaitError::ForceTerminated);
            }

            let (mut wait_queue_guard, wait_queue) = (self.0, self.1);

            // ensures that the allocator won't context switch
            wait_queue_guard.threads.reserve(1);

            let timeout_opt = timeout.into_option();
            let timeouted = with_interrupts(|| {
                let wake_at = if let Some(timeout) = timeout_opt {
                    Some(thread.sleep_for_ms(timeout))
                } else {
                    thread.block_waiting();
                    None
                };

                let timeout = Timeout::from_option(wake_at);
                wait_queue_guard
                    .threads
                    .push((thread.clone(), reason, timeout.clone()));
                drop(wait_queue_guard);
                thread::current::yield_now();

                timeout.is_timeout()
            });

            let remove_self = || {
                let mut wait_queue = wait_queue.lock();
                let index = wait_queue
                    .threads
                    .iter()
                    .position(|(t, _, _)| Arc::ptr_eq(t, &thread));

                if let Some(i) = index {
                    // TODO: should we swap_remove?
                    wait_queue.threads.swap_remove(i);
                }
            };

            if thread.should_terminate() {
                remove_self();
                return Err(WaitError::ForceTerminated);
            }

            if timeouted {
                remove_self();
                return Err(WaitError::Timeout);
            }

            Ok(())
        })
    }
}

impl<const AVERAGE: usize, Reason> PendingWait<'_, AVERAGE, Reason, Option<NonZero<u64>>> {
    /// Applies the [`PendingWait`], causing the current thread to sleep for at most `timeout_after` ms if Some, until a wake operation on the [`WaitQueue`] happens.
    pub fn enter_wait(
        self,
        reason: Reason,
        timeout_after: Option<NonZero<u64>>,
    ) -> Result<(), WaitError> {
        self.enter_wait_inner(reason, timeout_after)
    }
}
impl<const AVERAGE: usize, Reason> PendingWait<'_, AVERAGE, Reason, ()> {
    /// Applies the [`PendingWait`], causing the current thread to sleep until a wake operation on the [`WaitQueue`] happens.
    pub fn enter_wait(self, reason: Reason) -> Result<(), WaitError> {
        self.enter_wait_inner(reason, ())
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

trait TimeoutType: Clone {
    fn is_timeout(&self) -> bool;
    fn into_option(self) -> Option<NonZero<u64>>;
    fn from_option(opt: Option<NonZero<u64>>) -> Self;
}

impl TimeoutType for () {
    fn is_timeout(&self) -> bool {
        false
    }
    fn into_option(self) -> Option<NonZero<u64>> {
        None
    }
    fn from_option(opt: Option<NonZero<u64>>) -> Self {
        _ = opt;
        ()
    }
}

impl TimeoutType for Option<NonZero<u64>> {
    fn is_timeout(&self) -> bool {
        self.is_some_and(|wake_at| wake_at.get() <= time!(ms))
    }
    fn into_option(self) -> Option<NonZero<u64>> {
        self
    }
    fn from_option(opt: Option<NonZero<u64>>) -> Self {
        opt
    }
}

/// A wait queue for waiting on threads given a condition,
/// and waking them up when the condition is met or when the queue is destroyed.
/// the order of the threads is not guaranteed.
///
/// [`AVERAGE`] is the average number of threads that will be stored in the wait queue, used to avoid heap allocations.
/// [`Reason`] is the reason type for waiting.
#[derive(Debug)]
pub struct WaitQueue<
    const AVERAGE: usize = MIN_WAIT_THREADS,
    Reason = (),
    Timeout: TimeoutType = (),
> {
    threads: SmallVec<[(ArcThread, Reason, Timeout); AVERAGE]>,
}

/// [`WaitQueue`] but timeouts are available
pub type WaitQueueWithTimeout<const AVERAGE: usize, Reason> =
    WaitQueue<AVERAGE, Reason, Option<NonZero<u64>>>;

impl<const AVERAGE: usize, Reason, Timeout: TimeoutType> WaitQueue<AVERAGE, Reason, Timeout> {
    /// Creates a new wait queue.
    pub const fn new() -> Self {
        Self {
            threads: SmallVec::new_const(),
        }
    }

    /// Wakes all threads in the wait queue.
    pub fn wake_all(&mut self) {
        for (thread, _, _) in self.threads.drain(..) {
            thread.wake_up();
        }
    }

    /// Wakes all threads in the wait queue that satisfy the given condition.
    pub fn wake_on_condition(&mut self, mut condition: impl FnMut(&mut Reason) -> bool) {
        self.threads.retain(|(thread, reason, wake_at)| {
            // Will wake up by timeout
            if wake_at.is_timeout() {
                return false;
            }

            if condition(reason) {
                thread.wake_up();
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
            self.threads.retain(|(thread, reason, wake_at)| {
                if wake_at.is_timeout() {
                    return false;
                }

                if condition(reason) {
                    thread.wake_up();
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
}

impl<const AVERAGE: usize, Reason: Eq> WaitQueue<AVERAGE, Reason> {
    /// Wakes all threads in the wait queue that are waiting for the given reason.
    pub fn wake_equals(&mut self, reason: &Reason) {
        self.wake_on_condition(|r| r == reason)
    }
}

impl<const AVERAGE: usize, Reason, Timeout: TimeoutType> Drop
    for WaitQueue<AVERAGE, Reason, Timeout>
{
    fn drop(&mut self) {
        self.wake_all();
    }
}
