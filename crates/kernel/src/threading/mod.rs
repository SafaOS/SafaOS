pub mod cpu_context;
pub mod expose;
mod queue;
pub mod resources;
pub mod task;
#[cfg(test)]
mod tests;

/// Process ID, a unique identifier for a process (task)
pub type Pid = u32;

use core::cell::SyncUnsafeCell;
use core::ptr::NonNull;

use alloc::sync::Arc;
use lazy_static::lazy_static;
use safa_utils::{abi::raw::processes::AbiStructures, make_path};

use crate::threading::cpu_context::{ContextPriority, ContextStatus, Thread};
use crate::threading::queue::{TaskQueue, ThreadQueue};
use crate::utils::locks::RwLock;
use crate::utils::types::Name;
use crate::{VirtAddr, time};
use alloc::boxed::Box;
use slab::Slab;
use task::{Task, TaskInfo};

use crate::{
    arch::threading::{CPUStatus, restore_cpu_status},
    debug,
    memory::paging::PhysPageTable,
};

static TIMESLICES_LEFT: SyncUnsafeCell<u32> = SyncUnsafeCell::new(0);

/// Subtracts one timeslice from the current context's timeslices passed.
/// Returns `true` if the current context has finished all of its timeslices.
unsafe fn timeslices_sub_finished() -> bool {
    let ptr = TIMESLICES_LEFT.get();
    unsafe {
        if *ptr < 1 {
            *ptr = 0;
            true
        } else {
            *ptr -= 1;
            false
        }
    }
}

pub struct Scheduler {
    threads_queue: ThreadQueue,
    tasks_queue: TaskQueue,
    pids: Slab<()>,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            threads_queue: ThreadQueue::new(),
            tasks_queue: TaskQueue::new(),
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

        let (task, root_thread) = Task::new(
            Name::try_from(name).expect("initial process name too long"),
            0,
            0,
            cwd,
            page_table,
            context,
            VirtAddr::null(),
            ContextPriority::Medium,
            false,
        );

        unsafe {
            let status = root_thread.context().cpu_status();
            self::add(task, root_thread);

            debug!(Scheduler, "INITED ...");
            restore_cpu_status(status.as_ref())
        }
    }

    #[inline(always)]
    fn current_thread(&self) -> &Arc<Thread> {
        unsafe { self.threads_queue.current().unwrap_unchecked() }
    }

    /// context switches into next task, takes current context outputs new context
    /// returns the new context and a boolean indicating if the address space has changed
    /// if the address space has changed, please copy the context to somewhere accessible first
    pub unsafe fn switch(
        &mut self,
        current_status: CPUStatus,
    ) -> (NonNull<CPUStatus>, ContextPriority, bool) {
        unsafe {
            let current_thread = self.current_thread();
            let current_task = current_thread.task();
            let current_pid = current_task.pid();

            current_thread.context().set_cpu_status(current_status);

            while let Some(thread) = self.threads_queue.advance_circular() {
                if thread.is_dead() {
                    continue;
                }

                let context = thread.context();
                let status = context.status();

                let mut choose_context = move |set_runnable: bool| {
                    let task = thread.task();
                    debug_assert!(
                        task.is_alive(),
                        "thread didn't get marked as dead when Task was killed..."
                    );

                    if set_runnable {
                        context.set_status(ContextStatus::Runnable);
                    }
                    let task_pid = task.pid();
                    let address_space_changed = task_pid != current_pid;

                    let priority = context.priority();
                    let cpu_status = context.cpu_status();
                    (cpu_status, priority, address_space_changed)
                };

                match status {
                    ContextStatus::Runnable => return choose_context(false),
                    ContextStatus::Sleeping(time) if { time <= time!(ms) } => {
                        return choose_context(true);
                    }
                    ContextStatus::Sleeping(_) => continue,
                }
            }

            // TODO: fallback to the idle thread? for now the idle thread is just a part of the queue
            unreachable!("context switch failed")
        }
    }

    /// appends a task to the end of the scheduler taskes list
    /// returns the pid of the added task
    fn add_task(&mut self, task: Arc<Task>, root_thread: Arc<Thread>) -> Pid {
        let pid = self.pids.insert(()) as Pid;
        unsafe { task.set_pid(pid) };

        self.tasks_queue.push_back(task.clone());
        self.threads_queue.push_back(root_thread);

        let name = task.name();

        debug!(Scheduler, "Task {} ({}) ADDED", pid, name);
        pid
    }

    /// appends a thread to the end of the scheduler threads list
    /// returns the tid of the added thread
    fn add_thread(&mut self, thread: Arc<Thread>) {
        self.threads_queue.push_back(thread);
    }

    /// finds a task where executing `condition` on returns true and returns it
    fn find<C>(&self, condition: C) -> Option<&Task>
    where
        C: Fn(&Task) -> bool,
    {
        for task in self.tasks_queue.iter() {
            if condition(task) {
                return Some(&**task);
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
        for task in self.tasks_queue.iter() {
            then(task);
        }
    }

    /// attempt to remove a task where executing `condition` on returns true, returns the removed task info
    pub fn remove(&mut self, condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
        let task = self.tasks_queue.remove_where(|task| condition(task));
        if let Some(ref task) = task {
            self.threads_queue
                .remove_where(|thread| thread.task().pid() == task.pid());
        }
        let result = task.map(|task| TaskInfo::from(&*task));

        if let Some(ref info) = result {
            self.pids.remove(info.pid as usize);
        }
        result
    }

    #[inline(always)]
    /// whether or not has been properly initialized using `init`
    pub fn inited(&self) -> bool {
        self.tasks_queue.len() > 0
    }
}

pub(super) unsafe fn before_thread_yield() {
    unsafe {
        *TIMESLICES_LEFT.get() = 0;
    }
}

#[inline(always)]
/// performs a context switch using the scheduler, switching to the next task context
/// to be used
/// returns the new context and a boolean indicating if the address space has changed
/// if the address space has changed, please copy the context to somewhere accessible first
///
/// returns None if the scheduler is not yet initialized or nothing is supposed to be switched to
pub fn swtch(context: CPUStatus) -> Option<(NonNull<CPUStatus>, bool)> {
    if !unsafe { timeslices_sub_finished() } {
        return None;
    }

    match SCHEDULER.try_write().filter(|s| s.inited()) {
        Some(mut scheduler) => unsafe {
            let (cpu_status, priority, address_space_changed) = scheduler.switch(context);
            *TIMESLICES_LEFT.get() = priority.timeslices();

            Some((cpu_status, address_space_changed))
        },
        _ => None,
    }
}

lazy_static! {
    static ref SCHEDULER: RwLock<Scheduler> = RwLock::new(Scheduler::new());
}

/// Returns a static reference to the current task
/// # Safety
/// Safe because the current Task is always alive as long as there is code executing
pub fn this_task() -> Arc<Task> {
    this_thread().task().clone()
}

/// Returns a static reference to the current task
/// # Safety
/// Safe because the current Thread is always alive as long as there is code executing
pub fn this_thread() -> Arc<Thread> {
    let read = SCHEDULER.read();
    let curr = read.current_thread();
    curr.clone()
}

/// acquires lock on scheduler and finds a task where executing `condition` on returns true and returns the result of `map` on that task
pub fn find<C, M, R>(condition: C, map: M) -> Option<R>
where
    C: Fn(&Task) -> bool,
    M: Fn(&Task) -> R,
{
    let schd = SCHEDULER.read();
    schd.find(condition).map(map)
}

/// acquires lock on scheduler
/// executes `then` on each task
pub fn for_each<T>(then: T)
where
    T: FnMut(&Task),
{
    SCHEDULER.read().for_each(then)
}

/// acquires lock on scheduler and adds a task to it
fn add(task: Arc<Task>, root_thread: Arc<Thread>) -> Pid {
    SCHEDULER.write().add_task(task, root_thread)
}

/// acquires lock on scheduler and adds a thread to it
fn add_thread(thread: Arc<Thread>) {
    SCHEDULER.write().add_thread(thread)
}

/// returns the result of `then` if a task was found
/// acquires lock on scheduler and removes a task from it where `condition` on the task returns true
fn remove(condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
    SCHEDULER.write().remove(condition)
}
