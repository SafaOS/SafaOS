pub mod expose;
pub mod resources;
pub mod task;

pub type Pid = usize;

use core::arch::asm;
use lazy_static::lazy_static;
use safa_utils::make_path;

use crate::utils::types::Name;
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use slab::Slab;
use spin::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use task::{Task, TaskInfo, TaskMetadata, TaskState};

use crate::{
    arch::threading::{restore_cpu_status, CPUStatus},
    debug,
    memory::paging::PhysPageTable,
    utils::alloc::LinkedList,
};

pub struct Scheduler {
    tasks: LinkedList<Rc<Task>>,
    pids: Slab<()>,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: LinkedList::new(),
            pids: Slab::new(),
        }
    }

    #[inline]
    /// inits the scheduler
    pub unsafe fn init(function: fn() -> !, name: &str) -> ! {
        debug!(Scheduler, "initing ...");
        asm!("cli");
        let mut page_table = PhysPageTable::from_current();
        let context =
            CPUStatus::create(&mut page_table, &[], &[], function as usize, false).unwrap();
        let cwd = Box::new(make_path!("ram", "").into_owned().unwrap());

        let task = Task::new(
            Name::try_from(name).expect("initial process name too long"),
            0,
            0,
            cwd,
            page_table,
            context,
            0,
            TaskMetadata::default(),
        );
        self::add(task);

        // getting the context of the first task
        // like this so the scheduler read lock is released
        let context = *self::current().context();

        debug!(Scheduler, "INITED ...");
        unsafe { restore_cpu_status(&context) }
    }

    #[inline(always)]
    fn current(&self) -> &Rc<Task> {
        unsafe { self.tasks.current().unwrap_unchecked() }
    }

    /// context switches into next task, takes current context outputs new context
    pub unsafe fn switch(&mut self, context: CPUStatus) -> CPUStatus {
        unsafe { asm!("cli") }

        self.current().set_context(context);
        for task in self.tasks.continue_iter() {
            if task.is_alive() {
                break;
            }
        }

        *self.current().context()
    }

    /// appends a task to the end of the scheduler taskes list
    /// returns the pid of the added task
    fn add_task(&mut self, mut task: Task) -> usize {
        let pid = self.pids.insert(());
        task.pid = pid;
        self.tasks.push(Rc::new(task));

        debug!(
            Scheduler,
            "Task {} ({}) ADDED",
            pid,
            self.tasks.last().unwrap().name()
        );
        pid
    }

    /// finds a task where executing `condition` on returns true and returns it
    fn find<C>(&self, condition: C) -> Option<Rc<Task>>
    where
        C: Fn(&Task) -> bool,
    {
        for task in self.tasks.clone_iter() {
            if condition(task) {
                return Some(task.clone());
            }
        }

        None
    }

    /// iterates through all taskes and executes `then` on each of them
    /// executed on all taskes
    fn for_each<T>(&self, then: T)
    where
        T: Fn(&Task),
    {
        for task in self.tasks.clone_iter() {
            then(task);
        }
    }

    /// attempt to remove a task where executing `condition` on returns true, returns the removed task info
    pub fn remove(&mut self, condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
        let result = self
            .tasks
            .remove_where(|task| condition(task))
            .map(|task| TaskInfo::from(&*task));

        if let Some(ref info) = result {
            self.pids.remove(info.pid);
        }
        result
    }

    #[inline(always)]
    /// wether or not has been properly initialized using `init`
    pub fn inited(&self) -> bool {
        self.tasks.len() > 0
    }

    #[inline(always)]
    pub fn pids(&self) -> Vec<Pid> {
        let mut vec = Vec::with_capacity(self.pids.len());
        for task in self.tasks.clone_iter() {
            vec.push(task.pid);
        }

        vec
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

pub fn current() -> Rc<Task> {
    SCHEDULER.read().current().clone()
}

fn this_ptr() -> *const Task {
    let read = SCHEDULER.read();
    let curr = read.current();
    Rc::downgrade(curr).as_ptr()
}

/// Returns a static reference to the current task
/// # Safety
/// Safe because the current Task is always alive as long as there is code executing
pub fn this() -> &'static Task {
    unsafe { &*this_ptr() }
}

/// acquires lock on scheduler and finds a task where executing `condition` on returns true
fn find<C>(condition: C) -> Option<Rc<Task>>
where
    C: Fn(&Task) -> bool,
{
    SCHEDULER.read().find(condition)
}

/// acquires lock on scheduler
/// executes `then` on each task
fn for_each<T>(then: T)
where
    T: Fn(&Task),
{
    SCHEDULER.read().for_each(then)
}

/// acquires lock on scheduler and adds a task to it
fn add(task: Task) -> usize {
    SCHEDULER.write().add_task(task)
}

/// returns the result of `then` if a task was found
/// acquires lock on scheduler and removes a task from it where `condition` on the task returns true
fn remove(condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
    SCHEDULER.write().remove(condition)
}

pub fn schd() -> RwLockReadGuard<'static, Scheduler> {
    SCHEDULER.read()
}

/// Gets a readlock on the current task's states
/// # Safety
/// Safe because the task always is
#[inline(always)]
#[must_use]
pub fn this_state() -> RwLockReadGuard<'static, TaskState> {
    loop {
        match this().state() {
            Some(s) => return s,
            None => expose::thread_yeild(),
        }
    }
}

/// Gets a writelock on the current task's states
/// # Safety
/// Safe because the task always is
#[inline(always)]
#[must_use]
pub fn this_state_mut() -> RwLockWriteGuard<'static, TaskState> {
    loop {
        match this().state_mut() {
            Some(s) => return s,
            None => expose::thread_yeild(),
        }
    }
}
