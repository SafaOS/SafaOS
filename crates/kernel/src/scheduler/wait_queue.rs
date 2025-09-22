// Hack to allow timeouts as an option
#![allow(private_bounds)]

use core::num::NonZero;

use smallvec::SmallVec;

use crate::{thread::ArcThread, time};

const MIN_WAIT_THREADS: usize = 4;

trait TimeoutType {
    fn is_timeout(&self) -> bool;
}

impl TimeoutType for () {
    fn is_timeout(&self) -> bool {
        false
    }
}

impl TimeoutType for Option<NonZero<u64>> {
    fn is_timeout(&self) -> bool {
        self.is_some_and(|wake_at| wake_at.get() <= time!(ms))
    }
}

/// A wait queue for waiting on threads given a condition,
/// and waking them up when the condition is met or when the queue is destroyed.
/// the order of the threads is not guaranteed.
///
/// [`AVERAGE`] is the average number of threads that will be stored in the wait queue, used to avoid heap allocations.
/// [`Reason`] is the reason type for waiting.
#[derive(Debug, Clone)]
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

impl<const AVERAGE: usize, Reason> WaitQueue<AVERAGE, Reason, ()> {
    /// Pushes a thread and its sleep reason into the wait queue.
    /// also handles setting the thread's status to blocked and such.
    ///
    /// # Safety
    /// safe to call but may cause a deadlock if used incorrectly, please disable interrupts before calling this function,
    /// and drop all the locks
    pub fn push(&mut self, thread: ArcThread, reason: Reason) {
        thread.temp_block_forever();
        self.threads.push((thread, reason, ()));
    }
}

impl<const AVERAGE: usize, Reason> WaitQueue<AVERAGE, Reason, Option<NonZero<u64>>> {
    /// Pushes a thread and its sleep reason into the wait queue, with an optional timeout duration, returns the wake time of the thread if it was timed out.
    /// also handles setting the thread's status to blocked and such.
    /// # Safety
    /// safe to call but may cause a deadlock if used incorrectly, please disable interrupts before calling this function,
    /// and drop all the locks
    ///
    pub fn push(
        &mut self,
        thread: ArcThread,
        reason: Reason,
        duration: Option<NonZero<u64>>,
    ) -> Option<NonZero<u64>> {
        let wake_at = if let Some(ms) = duration {
            Some(thread.sleep_for_ms(ms))
        } else {
            thread.temp_block_forever();
            None
        };
        self.threads.push((thread, reason, wake_at));
        wake_at
    }
}

impl<const AVERAGE: usize, Reason, Timeout: TimeoutType> WaitQueue<AVERAGE, Reason, Timeout> {
    /// Creates a new wait queue.
    pub const fn new() -> Self {
        Self {
            threads: SmallVec::new_const(),
        }
    }

    pub fn len(&self) -> usize {
        self.threads.len()
    }

    /// Wakes all threads in the wait queue.
    pub fn wake_all(&mut self) {
        for (thread, _, wake_at) in self.threads.drain(..) {
            // Will wake up by timeout
            if wake_at.is_timeout() {
                continue;
            }

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

    /// Attempts to pop a thread from the wait queue and apply the given function to its reason, returning None if the queue is empty.
    pub fn try_pop_one<R>(&mut self, d: impl FnOnce(Reason) -> R) -> Option<R> {
        let (thread, reason, timeouted) = self.threads.pop()?;
        if timeouted.is_timeout() {
            return self.try_pop_one(d);
        }

        let results = Some(d(reason));
        thread.wake_up();
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
