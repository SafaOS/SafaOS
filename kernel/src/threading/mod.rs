pub mod expose;
pub mod resources;
pub mod task;

pub type Pid = usize;

use core::arch::asm;
use lazy_static::lazy_static;

use alloc::{string::String, vec::Vec};
use spin::RwLock;
use task::{Task, TaskInfo};

use crate::{
    arch::threading::{restore_cpu_status, CPUStatus},
    debug,
    memory::paging::PhysPageTable,
    utils::alloc::LinkedList,
};

pub struct Scheduler {
    tasks: LinkedList<Task>,
    next_pid: usize,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: LinkedList::new(),
            next_pid: 0,
        }
    }

    #[inline]
    /// inits the scheduler
    pub unsafe fn init(function: fn() -> !, name: &str) -> ! {
        debug!(Scheduler, "initing ...");
        asm!("cli");
        let mut page_table = PhysPageTable::from_current();
        let context = CPUStatus::create(&mut page_table, &[], function as usize, false).unwrap();

        let task = Task::new(
            String::from(name),
            0,
            0,
            String::from("ram:/"),
            page_table,
            context,
            0,
        );
        add_task(task);

        // getting the context of the first task
        // like this so the scheduler read lock is released
        let context = with_current(|task| task.context);

        debug!(Scheduler, "INITED ...");
        unsafe { restore_cpu_status(&context) }
    }

    /// gets a mutable reference to the current task
    fn current(&mut self) -> &mut Task {
        unsafe { self.tasks.current_mut().unwrap_unchecked() }
    }

    /// context switches into next task, takes current context outputs new context
    pub unsafe fn switch(&mut self, context: CPUStatus) -> CPUStatus {
        unsafe { asm!("cli") }

        self.current().context = context;
        for task in self.tasks.continue_iter() {
            if task.is_alive.load(core::sync::atomic::Ordering::Relaxed) {
                break;
            }
        }

        self.current().context
    }

    /// appends a task to the end of the scheduler taskes list
    /// returns the pid of the added task
    fn add_task(&mut self, mut task: Task) -> usize {
        let pid = self.next_pid;
        task.pid = pid;
        task.is_alive
            .store(true, core::sync::atomic::Ordering::Relaxed);
        self.next_pid += 1;
        self.tasks.push(task);

        debug!(
            Scheduler,
            "Task {} ({}) ADDED",
            pid,
            self.tasks.last().unwrap().name()
        );
        pid
    }

    /// executes `then` on the current task
    fn with_current<T, R>(&self, then: T) -> R
    where
        T: FnOnce(&Task) -> R,
    {
        unsafe { then(self.tasks.current().unwrap_unchecked()) }
    }

    /// finds a task where executing `condition` on returns true, then executes `then` on it
    /// returns the result of `then` if a task was found
    fn find<C, T, R>(&self, condition: C, mut then: T) -> Option<R>
    where
        C: Fn(&Task) -> bool,
        T: FnMut(&Task) -> R,
    {
        for task in self.tasks.clone_iter() {
            if condition(task) {
                return Some(then(task));
            }
        }

        None
    }

    /// Executes `then` on a all tasks and returns a vector of the results
    pub fn map<T, R>(&self, mut then: T) -> Vec<R>
    where
        T: FnMut(&Task) -> R,
    {
        let mut results = Vec::with_capacity(self.tasks.len());
        for task in self.tasks.clone_iter() {
            results.push(then(task));
        }
        results
    }

    /// iterates through all taskes and executes `then` on each of them
    /// executed on all taskes
    fn for_each<T>(&mut self, mut then: T)
    where
        T: FnMut(&mut Task),
    {
        for task in self.tasks.clone_iter_mut() {
            then(task);
        }
    }

    /// attempt to remove a task where executing `condition` on returns true, returns the removed task info
    pub fn remove(&mut self, condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
        self.tasks
            .remove_where(|task| condition(task))
            .map(|task| TaskInfo::from(&task))
    }

    #[inline(always)]
    /// wether or not has been properly initialized using `init`
    pub fn inited(&self) -> bool {
        self.tasks.len() > 0
    }
}

#[inline(always)]
/// peforms a context switch using the scheduler, switching to the next task context
/// to be used
pub fn swtch(context: CPUStatus) -> CPUStatus {
    if let Some(mut scheduler) = SCHEDULER.try_write().filter(|s| s.inited()) {
        unsafe { scheduler.switch(context) }
    } else {
        context
    }
}

lazy_static! {
    static ref SCHEDULER: RwLock<Scheduler> = RwLock::new(Scheduler::new());
}

/// acquires lock on scheduler and executes `then` on the current task
fn with_current<T, R>(then: T) -> R
where
    T: FnOnce(&Task) -> R,
{
    SCHEDULER.read().with_current(then)
}

/// acquires lock on scheduler and finds a task where executing `condition` on returns true, then executes `then` on it
/// returns the result of `then` if a task was found
fn find<C, T, R>(condition: C, then: T) -> Option<R>
where
    C: Fn(&Task) -> bool,
    T: FnMut(&Task) -> R,
{
    SCHEDULER.read().find(condition, then)
}

/// acquires lock on scheduler and executes `then` on a all tasks and returns a vector of the results
pub fn map<T, R>(then: T) -> Vec<R>
where
    T: FnMut(&Task) -> R,
{
    SCHEDULER.read().map(then)
}

/// acquires lock on scheduler
/// executes `then` on each task
fn for_each<T>(then: T)
where
    T: FnMut(&mut Task),
{
    SCHEDULER.write().for_each(then)
}

/// acquires lock on scheduler and adds a task to it
fn add_task(task: Task) -> usize {
    SCHEDULER.write().add_task(task)
}

/// acquires lock on scheduler and removes a task from it where `condition` on the task returns true
fn remove(condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
    SCHEDULER.write().remove(condition)
}
