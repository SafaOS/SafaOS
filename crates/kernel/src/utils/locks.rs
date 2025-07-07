use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use lock_api::{GuardSend, RawMutex, RawRwLock};
use spin::Lazy;

use crate::threading::expose::thread_yield;
pub const SPIN_AMOUNT: u32 = 10_000;

pub struct LockRawMutex(AtomicBool);

#[inline(always)]
fn lock_loop<T>(this: &T, try_lock: impl Fn(&T) -> bool) {
    let mut spin_count = 0;
    while !try_lock(this) {
        core::hint::spin_loop();
        spin_count += 1;
        if spin_count > SPIN_AMOUNT {
            thread_yield();
            spin_count = 0;
        }
    }
}

unsafe impl RawMutex for LockRawMutex {
    const INIT: Self = Self(AtomicBool::new(false));
    type GuardMarker = GuardSend;

    fn lock(&self) {
        lock_loop(self, Self::try_lock)
    }

    #[inline(always)]
    fn try_lock(&self) -> bool {
        self.0
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    #[inline(always)]
    fn is_locked(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}

pub struct LockRawRwLock(AtomicU32);
impl LockRawRwLock {
    pub const WRITER_BIT: u32 = 1 << 31;
}

unsafe impl RawRwLock for LockRawRwLock {
    const INIT: Self = Self(AtomicU32::new(0));
    type GuardMarker = GuardSend;

    fn lock_shared(&self) {
        lock_loop(self, Self::try_lock_shared)
    }

    fn lock_exclusive(&self) {
        lock_loop(self, Self::try_lock_exclusive)
    }

    fn try_lock_shared(&self) -> bool {
        let mut state = self.0.load(Ordering::Relaxed);
        loop {
            if state & Self::WRITER_BIT != 0 {
                return false;
            }

            match self.0.compare_exchange_weak(
                state,
                state + 1,
                Ordering::Acquire, // Sync when acquired
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(s) => state = s,
            }
        }
    }

    fn try_lock_exclusive(&self) -> bool {
        self.0
            .compare_exchange(0, Self::WRITER_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn is_locked(&self) -> bool {
        self.0.load(Ordering::Relaxed) != 0
    }

    fn is_locked_exclusive(&self) -> bool {
        self.0.load(Ordering::Relaxed) & Self::WRITER_BIT != 0
    }

    unsafe fn unlock_shared(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }

    unsafe fn unlock_exclusive(&self) {
        self.0.store(0, Ordering::Release);
    }
}

type MutexExt<T> = lock_api::Mutex<LockRawMutex, T>;
pub type MutexGuard<'a, T> = lock_api::MutexGuard<'a, LockRawMutex, T>;

type RwLockExt<T> = lock_api::RwLock<LockRawRwLock, T>;
pub type RwLockReadGuard<'a, T> = lock_api::RwLockReadGuard<'a, LockRawRwLock, T>;
pub type RwLockWriteGuard<'a, T> = lock_api::RwLockWriteGuard<'a, LockRawRwLock, T>;

pub type SpinRwLock<T> = spin::RwLock<T>;

#[derive(Debug)]
#[repr(transparent)]
pub struct Mutex<T>(MutexExt<T>);

#[derive(Debug)]
#[repr(transparent)]
pub struct RwLock<T>(RwLockExt<T>);

#[derive(Debug)]
#[repr(transparent)]
pub struct LazyLock<T>(Lazy<T>);

macro_rules! impl_common {
    ($name: ident) => {
        impl_common!($name, ${concat($name, Ext)});
    };
    ($name: ident, $name_ext: ident) => {
        impl<T> Deref for $name<T> {
            type Target = $name_ext<T>;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T> DerefMut for $name<T> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl<T> $name<T> {
            pub const fn new(inner: T) -> Self {
                Self(<Self as Deref>::Target::new(inner))
            }

            #[allow(unused)]
            pub fn get_mut(&mut self) -> &mut T {
                self.0.get_mut()
            }
        }
    };


}

impl_common!(Mutex);
impl_common!(RwLock);

impl<T> Mutex<T> {
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock()
    }
}

impl<T> RwLock<T> {
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        self.0.read()
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        self.0.write()
    }
}

impl<T> Deref for LazyLock<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<T> LazyLock<T> {
    pub const fn new(f: fn() -> T) -> Self {
        Self(Lazy::new(f))
    }
}
