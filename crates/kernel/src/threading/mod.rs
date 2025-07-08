pub mod cpu_context;
pub mod expose;
pub mod queue;
pub mod resources;
pub mod task;
#[cfg(test)]
mod tests;

/// Process ID, a unique identifier for a process (task)
pub type Pid = u32;

use core::cell::SyncUnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::AtomicBool;

use alloc::sync::Arc;
use lazy_static::lazy_static;
use safa_utils::{abi::raw::processes::AbiStructures, make_path};

use crate::threading::cpu_context::{ContextPriority, ContextStatus, Thread};
use crate::threading::queue::{TaskQueue, ThreadQueue};
use crate::utils::locks::{Mutex, MutexGuard, SpinRwLock};
use crate::utils::types::Name;
use crate::{VirtAddr, arch, time};
use alloc::boxed::Box;
use slab::Slab;
use task::{Task, TaskInfo};

use crate::{
    arch::threading::{CPUStatus, restore_cpu_status},
    debug,
    memory::paging::PhysPageTable,
};

#[derive(Debug)]
pub struct CPULocalStorage {
    schedule_queue: Mutex<ThreadQueue>,
    time_slices_left: SyncUnsafeCell<u32>,
}
impl CPULocalStorage {
    pub fn new(threads_queue: ThreadQueue) -> Self {
        Self {
            schedule_queue: Mutex::new(threads_queue),
            time_slices_left: SyncUnsafeCell::new(0),
        }
    }
}

unsafe impl Send for CPULocalStorage {}
unsafe impl Sync for CPULocalStorage {}

impl CPULocalStorage {
    pub fn get() -> &'static Self {
        unsafe { &*arch::threading::cpu_local_storage_ptr().cast() }
    }
    pub fn get_all() -> &'static [&'static Self] {
        unsafe { arch::threading::cpu_local_storages() }
    }
}

/// Subtracts one timeslice from the current context's timeslices passed.
/// Returns `true` if the current context has finished all of its timeslices.
unsafe fn timeslices_sub_finished() -> bool {
    let local = CPULocalStorage::get();
    let ptr = local.time_slices_left.get();
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
    tasks_queue: TaskQueue,
    pids: Slab<()>,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks_queue: TaskQueue::new(),
            pids: Slab::new(),
        }
    }

    /// inits the scheduler
    pub unsafe fn init(main_function: fn() -> !, idle_function: fn() -> !, name: &str) -> ! {
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
                VirtAddr::from(main_function as usize),
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
            crate::arch::disable_interrupts();
            let status = arch::threading::init_cpus(&task, idle_function);
            let status = status.as_ref();
            self::add(task, root_thread);
            SCHEDULER_INITED.store(true, core::sync::atomic::Ordering::Release);

            debug!(
                Scheduler,
                "INITED, jumping to: {:#x} with stack: {:#x} ...",
                status.at(),
                status.stack_at()
            );
            restore_cpu_status(status)
        }
    }

    /// context switches into next task, takes current context outputs new context
    /// returns the new context and a boolean indicating if the address space has changed
    /// if the address space has changed, please copy the context to somewhere accessible first
    pub unsafe fn switch(
        threads_queue: &mut ThreadQueue,
        current_status: CPUStatus,
    ) -> (NonNull<CPUStatus>, ContextPriority, bool) {
        unsafe {
            let queue = threads_queue;
            let current_thread = queue.current().unwrap();
            let current_context = current_thread.context();
            let current_task = current_thread.task();
            let current_pid = current_task.pid();

            current_context.set_cpu_status(current_status);

            if current_context.status() == ContextStatus::Running {
                current_context.set_status(ContextStatus::Runnable);
            }

            if !current_task.is_alive() {
                current_task
                    .schedule_cleanup
                    .store(true, core::sync::atomic::Ordering::SeqCst);
            }

            while let Some(thread) = queue.advance_circular() {
                let task = thread.task();

                let task_pid = task.pid();
                let address_space_changed = task_pid != current_pid;

                if thread.is_dead() {
                    continue;
                }

                let context = thread.context();
                let status = context.status();

                let mut choose_context = move || {
                    debug_assert!(
                        task.is_alive(),
                        "thread didn't get marked as dead when Task was killed..."
                    );

                    context.set_status(ContextStatus::Running);

                    let priority = context.priority();
                    let cpu_status = context.cpu_status();
                    (cpu_status, priority, address_space_changed)
                };

                match status {
                    ContextStatus::Runnable => return choose_context(),
                    ContextStatus::Sleeping(time) if { time <= time!(ms) } => {
                        return choose_context();
                    }
                    ContextStatus::Sleeping(_) => continue,
                    ContextStatus::Running => unreachable!(),
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
        self.add_thread(root_thread, None);

        let name = task.name();

        debug!(Scheduler, "Task {} ({}) ADDED", pid, name);
        pid
    }

    /// appends a thread to the end of the scheduler threads list
    /// returns the tid of the added thread
    ///
    /// by default (if `cpu` is None) chooses the least full CPU to append to otherwise if CPU is Some(i) and i is a valid CPU index, chooses that CPU
    /// use Some(0) to append to the boot CPU
    fn add_thread(&mut self, thread: Arc<Thread>, cpu: Option<usize>) {
        let cpu_locals = CPULocalStorage::get_all();

        let (mut thread_queue, cpu_index) = if let Some(cpu) = cpu
            && let Some(local) = cpu_locals.get(cpu)
        {
            (local.schedule_queue.lock(), cpu)
        } else {
            let mut least_full: Option<(MutexGuard<'static, ThreadQueue>, usize)> = None;

            for (index, cpu_local) in cpu_locals.iter().enumerate() {
                let queue = cpu_local.schedule_queue.lock();
                if least_full
                    .as_ref()
                    .is_none_or(|(curr_queue, _)| curr_queue.len() > queue.len())
                {
                    let is_empty = queue.len() == 1;
                    least_full = Some((queue, index));
                    if is_empty {
                        break;
                    }
                }
            }
            least_full.expect("no CPUs were found")
        };

        let cid = unsafe { thread.context().cid() };
        let pid = thread.task().pid();

        thread_queue.push_back(thread);
        debug!(
            Scheduler,
            "Thread {cid} added for Task {pid}, CPU: {cpu_index}"
        );
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
        let task = self.tasks_queue.remove_where(|task| condition(task))?;

        let cpu_locals = CPULocalStorage::get_all();
        for cpu_local in cpu_locals {
            let mut queue = cpu_local.schedule_queue.lock();
            queue.remove_where(|thread| {
                if thread.task().pid() == task.pid() {
                    assert!(thread.is_dead());

                    let context = unsafe { thread.context() };
                    // wait for thread to exit
                    while context.status() == ContextStatus::Running {
                        core::hint::spin_loop();
                    }
                    true
                } else {
                    false
                }
            });
        }

        let (info, page_table) = task.cleanup();
        drop(page_table);

        self.pids.remove(info.pid as usize);
        Some(info)
    }
}

pub static SCHEDULER_INITED: AtomicBool = AtomicBool::new(false);

pub(super) unsafe fn before_thread_yield() {
    unsafe {
        *CPULocalStorage::get().time_slices_left.get() = 0;
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
    // FIXME: i relay on short circuit here which might not be a good idea
    if !SCHEDULER_INITED.load(core::sync::atomic::Ordering::Acquire)
        || !unsafe { timeslices_sub_finished() }
    {
        return None;
    }

    let _write_guard = SCHEDULER.write();

    let local = CPULocalStorage::get();
    let mut queue = local.schedule_queue.lock();

    unsafe {
        let (cpu_status, priority, address_space_changed) = Scheduler::switch(&mut queue, context);
        *local.time_slices_left.get() = priority.timeslices();

        Some((cpu_status, address_space_changed))
    }
}

lazy_static! {
    static ref SCHEDULER: SpinRwLock<Scheduler> = SpinRwLock::new(Scheduler::new());
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
    let curr_queue = CPULocalStorage::get().schedule_queue.lock();
    curr_queue
        .current()
        .expect("no current thread found for the current CPU")
        .clone()
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
fn add_thread(thread: Arc<Thread>, cpu: Option<usize>) {
    SCHEDULER.write().add_thread(thread, cpu)
}

/// returns the result of `then` if a task was found
/// acquires lock on scheduler and removes a task from it where `condition` on the task returns true
fn remove(condition: impl Fn(&Task) -> bool) -> Option<TaskInfo> {
    SCHEDULER.write().remove(condition)
}
