pub mod cpu_context;
pub mod expose;
pub mod resources;
pub mod task;
#[cfg(test)]
mod tests;

/// Process ID, a unique identifier for a process (task)
pub type Pid = u32;

use core::marker::PhantomData;
use core::ptr::NonNull;

use lazy_static::lazy_static;
use safa_utils::{abi::raw::processes::AbiStructures, make_path};

use crate::threading::cpu_context::ContextStatus;
use crate::threading::task::CPUContexts;
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

struct TaskNode {
    inner: Task,
    next: Option<NonNull<TaskNode>>,
    prev: Option<NonNull<TaskNode>>,
}

struct SchedulerTaskQueue {
    head: Option<NonNull<TaskNode>>,
    current: Option<NonNull<TaskNode>>,
    tail: Option<NonNull<TaskNode>>,
    len: usize,
}

impl SchedulerTaskQueue {
    pub const fn new() -> Self {
        Self {
            head: None,
            current: None,
            tail: None,
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// Advances the current task pointer in a circular manner, returning a reference to the next task which is now the current task.
    /// returns None only if the queue is empty.
    pub fn advance_circular(&mut self) -> Option<&Task> {
        if let Some(current) = self.current.take() {
            let current_ref = unsafe { current.as_ref() };
            if let Some(next) = current_ref.next {
                self.current = Some(next);
            } else {
                self.current = self.head;
            }
            self.current()
        } else {
            None
        }
    }

    pub fn current(&self) -> Option<&Task> {
        if let Some(current) = self.current {
            let current_ref = unsafe { current.as_ref() };
            Some(&current_ref.inner)
        } else {
            None
        }
    }

    pub fn push_back(&mut self, task: Task) {
        let node = Box::new(TaskNode {
            inner: task,
            next: None,
            prev: None,
        });

        let mut node_ptr = NonNull::new(Box::into_raw(node)).unwrap();
        let node_ref = unsafe { node_ptr.as_mut() };

        if let Some(mut tail) = self.tail {
            let tail_ref = unsafe { tail.as_mut() };
            debug_assert!(tail_ref.next.is_none());

            tail_ref.next = Some(node_ptr);
            node_ref.prev = Some(tail);
        } else {
            self.head = Some(node_ptr);
        }

        if self.current.is_none() {
            self.current = Some(node_ptr);
        }
        self.tail = Some(node_ptr);

        self.len += 1;
    }

    pub fn tail(&self) -> Option<&Task> {
        if let Some(tail) = self.tail {
            let tail_ref = unsafe { tail.as_ref() };
            Some(&tail_ref.inner)
        } else {
            None
        }
    }

    pub fn iter<'a>(&'a self) -> SchedulerTaskIter<'a> {
        SchedulerTaskIter {
            queue: PhantomData,
            current: self.head.map(|h| unsafe { h.as_ref() }),
        }
    }

    unsafe fn remove_raw_inner(&mut self, mut node_ptr: NonNull<TaskNode>) -> Box<TaskNode> {
        unsafe {
            let node = node_ptr.as_mut();
            let prev_ptr = node.prev.take();
            let next_ptr = node.next.take();

            let prev = prev_ptr.map(|mut prev| prev.as_mut());
            let next = next_ptr.map(|mut next| next.as_mut());

            if let Some(prev) = prev {
                prev.next = next_ptr;
            } else {
                self.head = next_ptr;
            }

            if let Some(next) = next {
                next.prev = prev_ptr;
            } else {
                self.tail = prev_ptr;
            }

            if let Some(current) = self.current
                && current == node_ptr
            {
                self.current = next_ptr;
            }

            self.len -= 1;
            Box::from_non_null(node_ptr)
        }
    }

    fn remove_where<F>(&mut self, mut predicate: F) -> Option<Box<TaskNode>>
    where
        F: FnMut(&Task) -> bool,
    {
        let mut current = self.head;
        while let Some(mut node_ptr) = current {
            let node = unsafe { node_ptr.as_mut() };
            current = node.next;
            if predicate(&node.inner) {
                return Some(unsafe { self.remove_raw_inner(node_ptr) });
            }
        }
        None
    }
}

struct SchedulerTaskIter<'a> {
    queue: PhantomData<&'a SchedulerTaskQueue>,
    current: Option<&'a TaskNode>,
}

impl<'a> Iterator for SchedulerTaskIter<'a> {
    type Item = &'a Task;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current.take()?;
        self.current = current.next.map(|node| unsafe { node.as_ref() });
        Some(&current.inner)
    }
}

pub struct Scheduler {
    tasks: SchedulerTaskQueue,
    pids: Slab<()>,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: SchedulerTaskQueue::new(),
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
            let current = self::this();
            let contexts = current.cpu_contexts();
            let context = contexts.current_mut().cpu_status().as_ref();

            debug!(Scheduler, "INITED ...");
            restore_cpu_status(context)
        }
    }

    #[inline(always)]
    fn current(&self) -> &Task {
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

            while let Some(task) = self.tasks.advance_circular() {
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
        self.tasks.push_back(task);

        debug!(
            Scheduler,
            "Task {} ({}) ADDED",
            pid,
            self.tasks.tail().unwrap().name()
        );
        pid
    }

    /// finds a task where executing `condition` on returns true and returns it
    fn find<C>(&self, condition: C) -> Option<&Task>
    where
        C: Fn(&Task) -> bool,
    {
        for task in self.tasks.iter() {
            if condition(task) {
                return Some(task);
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
        for task in self.tasks.iter() {
            then(task);
        }
    }

    /// attempt to remove a task where executing `condition` on returns true, returns the removed task info
    pub fn remove(&mut self, condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
        let result = self
            .tasks
            .remove_where(|task| condition(task))
            .map(|task| TaskInfo::from(&task.inner));

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

fn this_ptr() -> *const Task {
    let read = SCHEDULER.read();
    let curr = read.current();
    curr
}

/// Returns a static reference to the current task
/// # Safety
/// Safe because the current Task is always alive as long as there is code executing
pub fn this() -> &'static Task {
    unsafe { &*this_ptr() }
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
fn add(task: Task) -> Pid {
    SCHEDULER.write().add_task(task)
}

/// returns the result of `then` if a task was found
/// acquires lock on scheduler and removes a task from it where `condition` on the task returns true
fn remove(condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
    SCHEDULER.write().remove(condition)
}
