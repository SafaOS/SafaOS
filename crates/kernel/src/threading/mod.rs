pub mod cpu_context;
pub mod expose;
pub mod queue;
pub mod resources;
pub mod task;
#[cfg(test)]
mod tests;

/// Process ID, a unique identifier for a process (task)
pub type Pid = u32;

use core::cell::{SyncUnsafeCell, UnsafeCell};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize};

use alloc::sync::Arc;
use lazy_static::lazy_static;
use safa_utils::{abi::raw::processes::AbiStructures, make_path};

use crate::threading::cpu_context::{ContextPriority, ContextStatus, Thread, ThreadNode};
use crate::threading::expose::thread_yield;
use crate::threading::queue::TaskQueue;
use crate::utils::locks::{RwLock, SpinMutex};
use crate::utils::types::Name;
use crate::{VirtAddr, arch};
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
    current_thread: UnsafeCell<Arc<Thread>>,
    thread_node_queue: SpinMutex<(Box<ThreadNode>, *mut ThreadNode)>,
    threads_count: AtomicUsize,

    time_slices_left: SyncUnsafeCell<u32>,
}
impl CPULocalStorage {
    pub fn create(root_thread: Arc<Thread>) -> Box<Self> {
        let root_thread_node = ThreadNode::new(root_thread.clone());
        let mut root_thread_node = Box::new(root_thread_node);
        let root_thread_node_ptr = &raw mut *root_thread_node;

        let this = Self {
            current_thread: UnsafeCell::new(root_thread),
            thread_node_queue: SpinMutex::new((root_thread_node, root_thread_node_ptr)),
            threads_count: AtomicUsize::new(0),
            time_slices_left: SyncUnsafeCell::new(0),
        };

        Box::new(this)
    }
    /// Get the current thread
    pub fn current_thread(&self) -> Arc<Thread> {
        // safe because the current thread is only ever read by the current thread and modifieded by context switch
        unsafe { (*self.current_thread.get()).clone() }
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
        root_thread_node: &mut ThreadNode,
        current_thread_node: &mut *mut ThreadNode,
        current_thread_ptr: *mut Arc<Thread>,
        current_status: CPUStatus,
    ) -> (NonNull<CPUStatus>, ContextPriority, bool) {
        unsafe {
            let current_thread = &*current_thread_ptr;
            let current_cid = current_thread.cid();
            let current_context = current_thread.context();
            let current_task = current_thread.task();
            let current_pid = current_task.pid();

            current_context.set_cpu_status(current_status);

            if current_context.status().is_running() {
                current_context.set_status(ContextStatus::Runnable);
            }

            if !current_task.is_alive() {
                current_task
                    .schedule_cleanup
                    .store(true, core::sync::atomic::Ordering::SeqCst);
            }

            let mut current_node = *current_thread_node;
            // FIXME: a lil bit unsafe
            loop {
                let (
                    next_node,
                    next_is_head, /* BECAREFUL head should be treated specially, especially when muttating */
                ) = (*current_node)
                    .next
                    .as_deref_mut()
                    .map(|n| (n, false))
                    .unwrap_or((root_thread_node, true));

                {
                    let thread = next_node.thread();
                    let thread_cid = thread.cid();
                    let task = thread.task();

                    let task_pid = task.pid();
                    let address_space_changed = task_pid != current_pid;

                    if thread.is_dead() {
                        debug_assert!(!thread.is_removed());

                        // same cid, same thread, another thread must be the one to mark removal
                        if !address_space_changed && thread_cid == current_cid {
                            current_node = next_node;
                        } else {
                            thread.mark_removed();
                            let next = next_node.next.take();
                            if next_is_head {
                                *root_thread_node =
                                    /* all references to the node become invalid here... */
                                    *next.expect("no more threads to use as the head of the queue");
                            } else {
                                (*current_node).next = next;
                            }
                        }

                        continue;
                    }

                    let context = thread.context();
                    let status = context.status();

                    macro_rules! choose_context {
                        () => {{
                            context.set_status(ContextStatus::Running);

                            let priority = context.priority();
                            let cpu_status = context.cpu_status();
                            *current_thread_ptr = thread.clone();
                            *current_thread_node = next_node;
                            (cpu_status, priority, address_space_changed)
                        }};
                    }

                    match status {
                        ContextStatus::Runnable => return choose_context!(),
                        ContextStatus::Blocked(reason) if reason.block_lifted() => {
                            return choose_context!();
                        }
                        ContextStatus::Blocked(_) => {}
                        ContextStatus::Running => unreachable!(),
                    }

                    current_node = next_node;
                }
            }
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

        let (cpu_local, cpu_index) = if let Some(cpu) = cpu
            && let Some(local) = cpu_locals.get(cpu)
        {
            (local, cpu)
        } else {
            let mut least_full = None;
            for (index, cpu_local) in cpu_locals.iter().enumerate() {
                let threads_amount = cpu_local
                    .threads_count
                    .load(core::sync::atomic::Ordering::Acquire);

                if least_full.is_none_or(|(amount, _, _)| amount > threads_amount) {
                    let is_empty = threads_amount == 1;
                    least_full = Some((threads_amount, cpu_local, index));
                    if is_empty {
                        break;
                    }
                }
            }
            let (_, cpu_local, index) = least_full.expect("no CPUs were found");
            (cpu_local, index)
        };

        let mut queue_lock = cpu_local.thread_node_queue.lock();
        let (root_thread, _) = &mut *queue_lock;

        let cid = thread.cid();
        let pid = thread.task().pid();

        ThreadNode::push_front(root_thread, thread);
        cpu_local
            .threads_count
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        debug!(
            Scheduler,
            "Thread {cid} added for Task {pid}, CPU: {cpu_index}"
        );
    }

    /// finds a task where executing `condition` on returns true and returns it
    fn find<C>(&self, condition: C) -> Option<&Arc<Task>>
    where
        C: Fn(&Task) -> bool,
    {
        for task in self.tasks_queue.iter() {
            if condition(task) {
                return Some(&task);
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

        for thread in &*task.threads.lock() {
            assert!(thread.is_dead());
            // wait for thread to exit before removing
            // will be removed on context switch at some point
            while !thread.is_removed() {
                // --> thread is removed on thread yield by the scheduler as a part of the thread list iteration
                // one thread yield should be enough
                thread_yield();
                // however maybe the thread list is in another CPU...
                core::hint::spin_loop();
            }
        }

        let (info, page_table) = task.cleanup();
        drop(page_table);

        self.pids.remove(info.pid as usize);
        Some(info)
    }
}

pub static SCHEDULER_INITED: AtomicBool = AtomicBool::new(false);

/// Scheduler should be initialized first
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
    if !SCHEDULER_INITED.load(core::sync::atomic::Ordering::Acquire) {
        return None;
    }

    if !unsafe { timeslices_sub_finished() } {
        return None;
    }

    let local = CPULocalStorage::get();

    let mut queue_lock = local.thread_node_queue.try_lock()?;
    let (root_thread_node, current_thread_node_ptr) = &mut *queue_lock;

    unsafe {
        let (cpu_status, priority, address_space_changed) = Scheduler::switch(
            &mut **root_thread_node,
            current_thread_node_ptr,
            local.current_thread.get(),
            context,
        );
        *local.time_slices_left.get() = priority.timeslices();

        Some((cpu_status, address_space_changed))
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
    CPULocalStorage::get().current_thread()
}

/// acquires lock on scheduler and finds a task where executing `condition` on returns true and returns the result of `map` on that task
pub fn find<C, M, R>(condition: C, map: M) -> Option<R>
where
    C: Fn(&Task) -> bool,
    M: FnMut(&Arc<Task>) -> R,
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
