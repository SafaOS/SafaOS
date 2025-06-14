//! This module contains stuff related to polling Drivers by the idle thread
use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::utils::locks::{RwLock, RwLockReadGuard};

pub trait PolledDriver: Send + Sync {
    fn run_every_ms(&self) -> usize {
        100
    }

    fn poll(&self);
}

lazy_static! {
    static ref EVE_TO_POLL: RwLock<Vec<&'static dyn PolledDriver>> = RwLock::new(Vec::new());
}

pub fn add_to_poll<T: PolledDriver>(driver: &'static T) {
    EVE_TO_POLL.write().push(driver);
}

pub fn read_poll() -> RwLockReadGuard<'static, Vec<&'static dyn PolledDriver>> {
    EVE_TO_POLL.read()
}
