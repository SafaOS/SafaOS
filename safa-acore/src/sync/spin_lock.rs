use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug)]
pub struct SpinLockRaw(AtomicBool);

impl SpinLockRaw {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    #[allow(unused)]
    #[inline(always)]
    /// Attempts to acquire a primitive SpinLock, returns true if the lock was acquired.
    pub fn try_lock(&self) -> bool {
        self.0
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    #[allow(unused)]
    /// Attempts to acquire a primitive SpinLock
    ///
    /// may do a false failure use [`Self::try_lock`].
    #[inline(always)]
    pub fn try_lock_weak(&self) -> bool {
        self.0
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    #[cfg(not(target_arch = "aarch64"))]
    /// Spins until it acquires the SpinLock.
    pub fn spin_lock(&self) {
        while !self.try_lock_weak() {
            while self.0.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    /// Spins until it acquires the SpinLock.
    pub fn spin_lock(&self) {
        loop {
            if self.try_lock() {
                return;
            }

            unsafe {
                core::arch::asm!(
                    // Wait for events (sev)
                    "wfe",
                    options(nostack),
                );
            }
        }
    }

    /// Unlocks a Spinlock acquired by the current CPU.
    pub unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // Sets event register to 1 which wakes wfe
            // wfe wakes immediately if event register is already 1 before clearing it
            core::arch::asm!("sev", options(nostack))
        };
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a IntSpinLock<T>,
    data: *mut T,
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        unsafe { self.lock.inner.unlock() };
    }
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.data }
    }
}

#[derive(Debug)]
/// A SpinLock that is not guraanteed to `lock()` and to `unlock()` while interrupts are disabled unlike [`SpinLockIrq`].
///
/// An interrupt SpinLock may deadlock if it was taken again from an interrupted context.
pub struct IntSpinLock<T> {
    inner: SpinLockRaw,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for IntSpinLock<T> {}
unsafe impl<T: Send> Send for IntSpinLock<T> {}

impl<T> IntSpinLock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            inner: SpinLockRaw::new(),
            data: UnsafeCell::new(data),
        }
    }

    #[inline(always)]
    pub const fn get(&self) -> *mut T {
        self.data.get()
    }

    #[inline(always)]
    pub const fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }

    #[inline(always)]
    pub fn lock<'s>(&'s self) -> SpinLockGuard<'s, T> {
        self.inner.spin_lock();
        SpinLockGuard {
            lock: self,
            data: self.get(),
        }
    }

    #[inline(always)]
    pub unsafe fn force_unlock(&self) {
        unsafe { self.inner.unlock() };
    }
}

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
