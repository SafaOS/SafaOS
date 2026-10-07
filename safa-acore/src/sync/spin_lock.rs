use super::{IntSpinLock, SpinLockGuard};

#[derive(Debug)]
/// A SpinLock that is guraanteed to `lock()` and to `unlock()` while interrupts are disabled..
///
/// No IRQ SpinLocks may open a door for deadlocks in case of memory ollocations and IPIs inside of it,
pub struct SpinLockIrq<T> {
    inner: IntSpinLock<T>,
}

impl<T> SpinLockIrq<T> {
    pub const fn new(data: T) -> Self {
        Self {
            inner: IntSpinLock::new(data),
        }
    }

    #[inline(always)]
    /// Locks running a function `f` without interrupts enabled.
    pub fn lock_no_irq<'s, R>(&'s self, f: impl FnOnce(SpinLockGuard<'s, T>) -> R) -> R {
        crate::arch::without_interrupts(|| f(self.inner.lock()))
    }

    #[inline(always)]
    pub unsafe fn force_unlock(&self) {
        unsafe { self.inner.force_unlock() };
    }
}
