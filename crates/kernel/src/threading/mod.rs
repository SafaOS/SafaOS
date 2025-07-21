pub mod cpu_context;
pub mod expose;
pub mod process;
pub mod queue;
pub mod resources;
#[cfg(test)]
mod tests;

/// Process ID, a unique identifier for a process (process)
pub type Pid = u32;

use core::cell::{SyncUnsafeCell, UnsafeCell};
use core::ptr::NonNull;
use core::sync::atomic::AtomicUsize;

use alloc::sync::Arc;
use lazy_static::lazy_static;
use safa_utils::make_path;

use crate::arch::without_interrupts;
use crate::threading::cpu_context::{ContextPriority, ContextStatus, Thread, ThreadNode};
use crate::threading::expose::thread_yield;
use crate::threading::queue::ProcessQueue;
use crate::utils::locks::{RwLock, SpinMutex};
use crate::utils::types::Name;
use crate::{VirtAddr, arch};
use alloc::boxed::Box;
use process::{Process, ProcessInfo};
use slab::Slab;

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
    pub fn new(root_thread: Arc<Thread>) -> Self {
        let root_thread_node = ThreadNode::new(root_thread.clone());
        let mut root_thread_node = Box::new(root_thread_node);
        let root_thread_node_ptr = &raw mut *root_thread_node;

        Self {
            current_thread: UnsafeCell::new(root_thread),
            thread_node_queue: SpinMutex::new((root_thread_node, root_thread_node_ptr)),
            threads_count: AtomicUsize::new(0),
            time_slices_left: SyncUnsafeCell::new(0),
        }
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
    processes_queue: ProcessQueue,
    pids: Slab<()>,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            processes_queue: ProcessQueue::new(),
            pids: Slab::new(),
        }
    }

    /// inits the scheduler
    pub unsafe fn init(main_function: fn() -> !, idle_function: fn() -> !, name: &str) -> ! {
        debug!(Scheduler, "initing ...");
        without_interrupts(|| {
            let page_table = unsafe { PhysPageTable::from_current() };
            let cwd = Box::new(make_path!("ram", "").into_owned().unwrap());

            let pid = SCHEDULER.write().add_pid();
            let (process, root_thread) = Process::create(
                Name::try_from(name).expect("initial process name too long"),
                pid,
                pid,
                VirtAddr::from(main_function as usize),
                cwd,
                &[],
                &[],
                unsafe { core::mem::zeroed() },
                page_table,
                VirtAddr::null(),
                None,
                ContextPriority::Medium,
                false,
                None,
            )
            .expect("failed to create Eve");

            unsafe {
                let status = arch::threading::init_cpus(&process, idle_function);
                let status_ref = status.as_ref();
                self::add(process, root_thread);
                *SCHEDULER_INITED.get() = true;

                debug!(
                    Scheduler,
                    "INITED, jumping to: {:#x} with stack: {:#x} ...",
                    status_ref.at(),
                    status_ref.stack_at()
                );
                restore_cpu_status(status_ref)
            }
        })
    }

    /// context switches into next process, takes current context outputs new context
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
            let current_context = current_thread
                .context()
                .expect("context is None before the thread is removed");
            let current_process = current_thread.process();
            let current_pid = current_process.pid();

            current_context.set_cpu_status(current_status);

            let mut status = current_thread.status_mut();
            if status.is_running() {
                *status = ContextStatus::Runnable;
            }
            drop(status);

            if !current_process.is_alive() {
                current_process
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
                    let process = thread.process();

                    let process_pid = process.pid();
                    let address_space_changed = process_pid != current_pid;

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

                    let mut status = thread.status_mut();

                    macro_rules! choose_context {
                        () => {{
                            *status = ContextStatus::Running;
                            let priority = thread.priority();

                            let context = thread.context_unchecked();
                            let cpu_status = context.cpu_status();
                            *current_thread_ptr = thread.clone();
                            drop(status);
                            *current_thread_node = next_node;
                            (cpu_status, priority, address_space_changed)
                        }};
                    }

                    match &*status {
                        ContextStatus::Runnable => return choose_context!(),
                        ContextStatus::Blocked(reason) if reason.block_lifted() => {
                            return choose_context!();
                        }
                        ContextStatus::Blocked(_) => {}
                        ContextStatus::Running => unreachable!(),
                    }

                    drop(status);
                    current_node = next_node;
                }
            }
        }
    }

    fn add_pid(&mut self) -> Pid {
        self.pids.insert(()) as Pid
    }

    /// appends a process to the end of the scheduler processes list
    /// returns the pid of the added process
    fn add_process(&mut self, process: Arc<Process>, root_thread: Arc<Thread>) -> Pid {
        let pid = process.pid();

        self.processes_queue.push_back(process.clone());
        self.add_thread(root_thread, None);

        let name = process.name();

        debug!(Scheduler, "Process {} ({}) PROCESS ADDED", pid, name);
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

        let cid = thread.cid();
        let pid = thread.process().pid();

        without_interrupts(
            /* lock scheduler without interrupts enabled so we don't lock ourself */
            move || {
                let mut queue_lock = cpu_local.thread_node_queue.lock();
                let (root_thread, _) = &mut *queue_lock;

                ThreadNode::push_front(root_thread, thread);
                cpu_local
                    .threads_count
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            },
        );

        debug!(
            Scheduler,
            "Thread {cid} added for process {pid}, CPU: {cpu_index}"
        );
    }

    /// finds a process where executing `condition` on returns true and returns it
    fn find<C>(&self, condition: C) -> Option<&Arc<Process>>
    where
        C: Fn(&Process) -> bool,
    {
        for process in self.processes_queue.iter() {
            if condition(process) {
                return Some(&process);
            }
        }

        None
    }

    /// iterates through all processes and executes `then` on each of them
    /// executed on all processes
    pub fn for_each<T>(&self, mut then: T)
    where
        T: FnMut(&Process),
    {
        for process in self.processes_queue.iter() {
            then(process);
        }
    }

    /// attempt to remove a process where executing `condition` on returns true, returns the removed process info
    pub fn remove(&mut self, condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
        let process = self
            .processes_queue
            .remove_where(|process| condition(process))?;

        for thread in &*process.threads.lock() {
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

        let (info, page_table) = process.cleanup();
        drop(page_table);

        self.pids.remove(info.pid as usize);
        Some(info)
    }
}

pub static SCHEDULER_INITED: SyncUnsafeCell<bool> = SyncUnsafeCell::new(false);

/// Scheduler should be initialized first
pub(super) unsafe fn before_thread_yield() {
    unsafe {
        *CPULocalStorage::get().time_slices_left.get() = 0;
    }
}

#[inline(always)]
/// performs a context switch using the scheduler, switching to the next process context
/// to be used
/// returns the new context and a boolean indicating if the address space has changed
/// if the address space has changed, please copy the context to somewhere accessible first
///
/// returns None if the scheduler is not yet initialized or nothing is supposed to be switched to
pub fn swtch(context: CPUStatus) -> Option<(NonNull<CPUStatus>, bool)> {
    if !unsafe { *SCHEDULER_INITED.get() } {
        return None;
    }

    if !unsafe { timeslices_sub_finished() } {
        return None;
    }

    let local = CPULocalStorage::get();

    let mut queue_lock = local.thread_node_queue.lock();
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

/// Returns a static reference to the current process
/// # Safety
/// Safe because the current process is always alive as long as there is code executing
pub fn this_process() -> Arc<Process> {
    this_thread().process().clone()
}

/// Returns a static reference to the current process
/// # Safety
/// Safe because the current Thread is always alive as long as there is code executing
pub fn this_thread() -> Arc<Thread> {
    CPULocalStorage::get().current_thread()
}

/// acquires lock on scheduler and finds a process where executing `condition` on returns true and returns the result of `map` on that process
pub fn find<C, M, R>(condition: C, map: M) -> Option<R>
where
    C: Fn(&Process) -> bool,
    M: FnMut(&Arc<Process>) -> R,
{
    let schd = SCHEDULER.read();
    schd.find(condition).map(map)
}

/// acquires lock on scheduler
/// executes `then` on each process
pub fn for_each<T>(then: T)
where
    T: FnMut(&Process),
{
    SCHEDULER.read().for_each(then)
}

/// acquires lock on scheduler and adds a process to it
fn add(process: Arc<Process>, root_thread: Arc<Thread>) -> Pid {
    SCHEDULER.write().add_process(process, root_thread)
}

/// acquires lock on scheduler and adds a thread to it
fn add_thread(thread: Arc<Thread>, cpu: Option<usize>) {
    SCHEDULER.write().add_thread(thread, cpu)
}

/// returns the result of `then` if a process was found
/// acquires lock on scheduler and removes a process from it where `condition` on the process returns true
fn remove(condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
    SCHEDULER.write().remove(condition)
}
