use core::{pin::Pin, task::Waker};

use alloc::{boxed::Box, sync::Arc};
use futures_util::task::ArcWake;
use libkernel::sync::IntSpinLock;

use crate::scheduler;

/// Represents a task
pub struct Task {
    pub future: IntSpinLock<Pin<Box<dyn Future<Output = ()>>>>,
}

impl Task {
    pub fn waker(self: Arc<Self>) -> Waker {
        futures_util::task::waker(self)
    }
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl ArcWake for Task {
    fn wake(self: Arc<Self>) {
        scheduler::wake_task(self);
    }
    fn wake_by_ref(arc_self: &Arc<Self>) {
        scheduler::wake_task(arc_self.clone());
    }
}

pub fn spawn<F: Future<Output = ()> + 'static + Send>(fut: F) {
    scheduler::new_task(Arc::new(Task {
        future: IntSpinLock::new(Box::pin(fut)),
    }))
}
