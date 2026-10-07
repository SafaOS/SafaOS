use core::task::Context;

use alloc::{collections::vec_deque::VecDeque, sync::Arc};
use libkernel::sync::IntSpinLock;

use crate::{percpu, task::Task};

pub static TASK_POLL: IntSpinLock<VecDeque<Arc<Task>>> = IntSpinLock::new(VecDeque::new());

percpu::define! {
    static CURRENT_TASK: Option<Arc<Task>> = None;
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

        let waker = task.clone().waker();
        let mut future = task.future.lock();
        _ = future.as_mut().poll(&mut Context::from_waker(&waker));
    }
}
