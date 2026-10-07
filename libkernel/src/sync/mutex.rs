use core::{
    cell::UnsafeCell,
    fmt::Debug,
    ops::{Deref, DerefMut},
    pin::Pin,
    ptr::NonNull,
    sync::atomic::{AtomicU8, Ordering},
    task,
};

use crate::rt::{WaitQueue, queue::Waiter};
#[cfg(test)]
mod tests;

const LOCKED: u8 = 0b01;
const WAITERS: u8 = 0b10;

#[derive(Debug)]
pub struct RawMutex {
    state: AtomicU8,
    wait_queue: WaitQueue,
}

const SPINS: usize = 100;

impl RawMutex {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
            wait_queue: WaitQueue::new(),
        }
    }

    #[allow(unused)]
    #[inline(always)]
    /// Attempts to acquire a primitive SpinLock, returns true if the lock was acquired.
    pub fn try_lock(&self) -> bool {
        (self.state.fetch_or(LOCKED, Ordering::Acquire) & LOCKED) == 0
    }

    /// Unlocks a Mutex acquired by the current task.
    pub unsafe fn unlock(&self) {
        if self
            .state
            .compare_exchange(LOCKED, 0, Ordering::Release, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }

        let queue = self.wait_queue.lock();
        if !queue.is_empty() {
            self.state.store(WAITERS, Ordering::Release);
            queue.notify_one();
        } else {
            self.state.store(0, Ordering::Release);
        }
    }

    #[inline]
    fn try_spinlock(&self) -> bool {
        for _ in 0..SPINS {
            if self.state.load(Ordering::Relaxed) & LOCKED == 0 && self.try_lock() {
                return true;
            }

            core::hint::spin_loop();
        }

        false
    }

    pub fn poll_lock(&self, waiter: Pin<&mut Waiter>, cx: &mut task::Context) -> task::Poll<()> {
        // TODO: Fair?
        if self.try_spinlock() {
            unsafe { self.wait_queue.lock().remove(waiter) };
            return task::Poll::Ready(());
        }

        let mut queue = self.wait_queue.lock();
        let state_was = self.state.fetch_or(WAITERS | LOCKED, Ordering::Acquire);
        if state_was & LOCKED == 0 {
            unsafe {
                queue.remove_and_then(waiter, |_, q| {
                    if q.is_empty() {
                        self.state.fetch_and(!WAITERS, Ordering::Relaxed);
                    }
                })
            };

            task::Poll::Ready(())
        } else {
            unsafe { queue.enqueue(waiter, cx) };
            task::Poll::Pending
        }
    }
}

/// An Async Mutex implementation.
pub struct Mutex<T> {
    raw: RawMutex,
    data: UnsafeCell<T>,
}

impl<T> Mutex<T> {
    /// Constructs a new Mutex.
    pub const fn new(data: T) -> Self {
        Self {
            raw: RawMutex::new(),
            data: UnsafeCell::new(data),
        }
    }

    const unsafe fn make_guard_unchecked(&self) -> MutexGuard<'_, T> {
        MutexGuard {
            lock: self,
            data: unsafe { NonNull::new_unchecked(self.data.get()) },
        }
    }

    /// Returns a [`Lock`] future that acquires a MutexGuard when polled.
    #[inline(always)]
    pub const fn lock(&self) -> Lock<'_, T> {
        Lock {
            mutex: Some(self),
            waiter: Waiter::new(),
        }
    }
}

/// Represents a MutexGuard for a [`Mutex`]::lock operation.
pub struct MutexGuard<'a, T> {
    lock: &'a Mutex<T>,
    data: NonNull<T>,
}

unsafe impl<'a, T: Send> Send for MutexGuard<'a, T> {}
unsafe impl<'a, T: Sync> Sync for MutexGuard<'a, T> {}

impl<'a, T: Debug> Debug for MutexGuard<'a, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (&**self).fmt(f)
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        unsafe { self.lock.raw.unlock() };
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { self.data.as_ref() }
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.data.as_mut() }
    }
}

pin_project_lite::pin_project! {
    /// A future that will acquire mutex once `.await`ed
    pub struct Lock<'a, T> {
        mutex: Option<&'a Mutex<T>>,
        #[pin]
        waiter: Waiter,
    }


    impl<'a, T> PinnedDrop for Lock<'a, T> {
        fn drop(this: Pin<&mut Self>) {
            let mutex = this.mutex;
            let project = this.project();
            let waiter = project.waiter;
            if let Some(mutex) = mutex {
                let queue = mutex.raw.wait_queue.lock();
                unsafe {
                    queue.remove_and_then(waiter, |n, q| if n.is_some() {
                        q.notify_one()
                    } else {
                        false
                    })
                };
            }
        }
    }
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<'a, T> Future for Lock<'a, T> {
    type Output = MutexGuard<'a, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        let mutex = self.mutex.expect("poll() called after Ready");

        let waiter = self.as_mut().project().waiter;

        match mutex.raw.poll_lock(waiter, cx) {
            task::Poll::Ready(()) => unsafe {
                self.get_unchecked_mut().mutex = None;
                task::Poll::Ready(mutex.make_guard_unchecked())
            },
            task::Poll::Pending => task::Poll::Pending,
        }
    }
}
