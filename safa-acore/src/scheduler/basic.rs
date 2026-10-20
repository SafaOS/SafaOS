use core::{cell::SyncUnsafeCell, task::Context};

use alloc::{collections::vec_deque::VecDeque, sync::Arc};
use libkernel::sync::IntSpinLock;

use crate::{percpu, task::Task};

pub static TASK_POLL: IntSpinLock<VecDeque<Arc<Task>>> = IntSpinLock::new(VecDeque::new());

percpu::define! {
    static CURRENT_TASK: SyncUnsafeCell<Option<Arc<Task>>> = SyncUnsafeCell::new(None);
}

#[inline(always)]
/// Attempts to get a reference to the current task.
///
/// # Safety: The reference is alive and points to the current task as long as the current task is alive and executing this code.
///
/// Therefore it isn't `Send`.
pub unsafe fn current_task_ref<'a>() -> Option<&'a Arc<Task>> {
    unsafe { (*CURRENT_TASK.get()).as_ref() }
}
pub fn new_task(task: Arc<Task>) {
    add_to_task_poll(task)
}

pub fn wake_task(task: Arc<Task>) {
    add_to_task_poll(task)
}

fn add_to_task_poll(task: Arc<Task>) {
    TASK_POLL.lock().push_back(task);
}

fn pop_task_poll() -> Option<Arc<Task>> {
    TASK_POLL.lock().pop_front()
}

pub fn schedule_loop() {
    loop {
        let Some(task) = pop_task_poll() else {
            core::hint::spin_loop();
            continue;
        };

        let task = unsafe { (*CURRENT_TASK.get()).insert(task) };
        let future = unsafe { &mut (*task.future.get()) };
        let waker = task.waker_ref();
        _ = future.as_mut().poll(&mut Context::from_waker(&waker));

        unsafe { *CURRENT_TASK.get() = None };
    }
}
