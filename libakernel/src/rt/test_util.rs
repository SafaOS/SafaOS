//! AI Generated test utils
extern crate std;

use core::{
    future::Future,
    pin::{Pin, pin},
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};
use std::{
    sync::Arc,
    task::Wake,
    thread::{self, Thread},
};

/// A waker that counts how many times it was woken.
pub struct CountWaker(AtomicUsize);

impl CountWaker {
    pub fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl Wake for CountWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Returns a fresh counting waker and its counter.
/// Every call gives a *distinct* waker (distinct `data()` pointer).
pub fn counting() -> (Arc<CountWaker>, Waker) {
    let c = Arc::new(CountWaker(AtomicUsize::new(0)));
    let w = Waker::from(c.clone());
    (c, w)
}

pub fn poll_with<F: Future + ?Sized>(f: Pin<&mut F>, w: &Waker) -> Poll<F::Output> {
    f.poll(&mut Context::from_waker(w))
}

struct ThreadWaker(Thread);
impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Minimal single-future executor: park until woken.
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        thread::park();
    }
}
