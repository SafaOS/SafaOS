//! This module contains stuff related to polling Drivers by the idle thread
use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::{thread, utils::locks::RwLock};

pub trait PolledDriver: Send + Sync {
    fn thread_name(&self) -> &'static str;
    #[inline(always)]
    /// The amount of dealy until poll is called again
    fn run_every_ms(&self) -> u64 {
        100
    }
    /// Executed every [`Self::run_every_ms`]
    fn poll(&self);
    fn poll_function(&self) -> ! {
        let sleep_ms = self.run_every_ms();
        loop {
            self.poll();
            thread::current::sleep_for_ms(sleep_ms);
        }
    }
}

lazy_static! {
    static ref EVE_TO_POLL: RwLock<Vec<&'static dyn PolledDriver>> = RwLock::new(Vec::new());
}

pub fn add_to_poll<T: PolledDriver>(driver: &'static T) {
    EVE_TO_POLL.write().push(driver);
}

pub fn take_poll() -> Vec<&'static dyn PolledDriver> {
    core::mem::take(&mut *EVE_TO_POLL.write())
}
