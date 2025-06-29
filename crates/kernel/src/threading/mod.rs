pub mod cpu_context;
pub mod expose;
pub mod resources;
pub mod task;
#[cfg(test)]
mod tests;

/// Process ID, a unique identifier for a process (task)
pub type Pid = u32;

use core::ptr::NonNull;

use lazy_static::lazy_static;
use safa_utils::{abi::raw::processes::AbiStructures, make_path};

use crate::threading::cpu_context::ContextStatus;
use crate::threading::task::CPUContexts;
use crate::utils::locks::{RwLock, RwLockReadGuard};
use crate::utils::types::Name;
use crate::{VirtAddr, time};
use alloc::{boxed::Box, rc::Rc};
use slab::Slab;
use task::{Task, TaskInfo};

use crate::{
    arch::threading::{CPUStatus, restore_cpu_status},
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
        unsafe {
            crate::arch::disable_interrupts();
        }
        let mut page_table = unsafe { PhysPageTable::from_current() };
        let context = unsafe {
            CPUStatus::create_root(
                &mut page_table,
                &[],
                &[],
                AbiStructures::default(),
                VirtAddr::from(function as usize),
                false,
            )
            .unwrap()
        };
        let cwd = Box::new(make_path!("ram", "").into_owned().unwrap());

        let task = Task::new(
            Name::try_from(name).expect("initial process name too long"),
            0,
            0,
            cwd,
            page_table,
            context,
            VirtAddr::null(),
            false,
        );
        self::add(task);

        unsafe {
            // getting the context of the first task
            // like this so the scheduler read lock is released
            let current = self::current();
            let contexts = current.cpu_contexts();
            let context = contexts.current_mut().cpu_status().as_ref();

            debug!(Scheduler, "INITED ...");
            restore_cpu_status(context)
        }
    }

    #[inline(always)]
    fn current(&self) -> &Rc<Task> {
        unsafe { self.tasks.current().unwrap_unchecked() }
    }

    /// from given cpu contexts choose a context to run
    /// if `set_current_status` is `Some`, set the current (first) context's cpu status to the given status
    ///
    /// if `pick_first` is `true`, pick the first context in the list otherwise pick the next context in the list (completely ignoring the first context)
    ///
    /// returns None if no context is available
    fn choose_context(
        cpu_contexts: &mut CPUContexts,
        set_current_status: Option<CPUStatus>,
        pick_first: bool,
    ) -> Option<NonNull<CPUStatus>> {
        if let Some(status) = set_current_status {
            cpu_contexts.set_current_cpu_status(status);
        }

        fn pick_context_inner(context: &mut cpu_context::Context) -> Option<NonNull<CPUStatus>> {
            match context.status() {
                ContextStatus::Runnable => Some(context.cpu_status()),
                ContextStatus::Sleeping(time) if { time <= time!(ms) } => {
                    context.set_status(ContextStatus::Runnable);
                    Some(context.cpu_status())
                }
                ContextStatus::Sleeping(_) => None,
            }
        }

        if pick_first {
            let current_context = cpu_contexts.current_mut();
            if let Some(cpu_status) = pick_context_inner(current_context) {
                return Some(cpu_status);
            }
        }

        while let Some(context) = cpu_contexts.advance() {
            if let Some(cpu_status) = pick_context_inner(context) {
                return Some(cpu_status);
            }
        }
        None
    }

    /// context switches into next task, takes current context outputs new context
    /// returns the new context and a boolean indicating if the address space has changed
    /// if the address space has changed, please copy the context to somewhere accessible first
    pub unsafe fn switch(&mut self, current_status: CPUStatus) -> (NonNull<CPUStatus>, bool) {
        unsafe {
            let current = self.current();
            if current.is_alive() {
                let cpu_contexts = current.cpu_contexts();
                if let Some(cpu_status) =
                    Self::choose_context(cpu_contexts, Some(current_status), false)
                {
                    return (cpu_status, false);
                }
            }

            for task in self.tasks.continue_iter() {
                if !task.is_alive() {
                    continue;
                }

                let cpu_contexts = task.cpu_contexts();
                if let Some(cpu_status) = Self::choose_context(cpu_contexts, None, true) {
                    return (cpu_status, true);
                }
            }

            unreachable!("context switch failed")
        }
    }

    /// appends a task to the end of the scheduler taskes list
    /// returns the pid of the added task
    fn add_task(&mut self, mut task: Task) -> Pid {
        let pid = self.pids.insert(()) as Pid;
        task.set_pid(pid);
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
    pub fn for_each<T>(&self, mut then: T)
    where
        T: FnMut(&Task),
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
            self.pids.remove(info.pid as usize);
        }
        result
    }

    #[inline(always)]
    /// whether or not has been properly initialized using `init`
    pub fn inited(&self) -> bool {
        self.tasks.len() > 0
    }
}

#[inline(always)]
/// performs a context switch using the scheduler, switching to the next task context
/// to be used
/// returns the new context and a boolean indicating if the address space has changed
/// if the address space has changed, please copy the context to somewhere accessible first
///
/// returns None if the scheduler is not yet initialized
pub fn swtch(context: CPUStatus) -> Option<(NonNull<CPUStatus>, bool)> {
    match SCHEDULER.try_write().filter(|s| s.inited()) {
        Some(mut scheduler) => Some(unsafe { scheduler.switch(context) }),
        _ => None,
    }
}

lazy_static! {
    static ref SCHEDULER: RwLock<Scheduler> = RwLock::new(Scheduler::new());
}

/// Returns a shared reference to the current task
/// opposite to `this()` which returns a static reference, this function returns a shared reference and typically used in unsafe code
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
pub fn find<C>(condition: C) -> Option<Rc<Task>>
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
fn add(task: Task) -> Pid {
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
