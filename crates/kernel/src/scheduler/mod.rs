pub mod resources;
#[cfg(test)]
mod tests;

use core::cell::{SyncUnsafeCell, UnsafeCell};
use core::ptr::NonNull;
use core::sync::atomic::AtomicUsize;

use crate::thread::{self, ContextPriority, ContextStatus, Thread, ThreadNode};
use crate::utils::path::make_path;
use alloc::sync::Arc;

use crate::arch::without_interrupts;
use crate::process::Process;
use crate::utils::locks::SpinLock;
use crate::utils::types::Name;
use crate::{VirtAddr, arch};
use alloc::boxed::Box;

pub mod process_list;

use crate::{
    arch::threading::{CPUStatus, restore_cpu_status},
    debug,
    memory::paging::PhysPageTable,
};

#[derive(Debug)]
pub struct CPULocalStorage {
    current_thread: UnsafeCell<Arc<Thread>>,
    thread_node_queue: SpinLock<(Box<ThreadNode>, *mut ThreadNode)>,
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
            thread_node_queue: SpinLock::new((root_thread_node, root_thread_node_ptr)),
            threads_count: AtomicUsize::new(0),
            time_slices_left: SyncUnsafeCell::new(0),
        }
    }
    /// Get the current thread
    pub fn current_thread(&self) -> Arc<Thread> {
        // safe because the current thread is only ever read by the current thread and modifieded by context switch
        unsafe { (*self.current_thread.get()).clone() }
    }
    /// Get a reference to the current thread
    pub fn current_thread_ref(&self) -> &Arc<Thread> {
        unsafe { &*self.current_thread.get() }
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

pub static SCHEDULER_INITED: SyncUnsafeCell<bool> = SyncUnsafeCell::new(false);

/// Scheduler should be initialized first
pub(super) unsafe fn before_thread_yield() {
    unsafe {
        *CPULocalStorage::get().time_slices_left.get() = 0;
    }
}

/// context switches into next process, takes current context outputs new context
/// returns the new context and a boolean indicating if the address space has changed
unsafe fn switch_inner(
    root_thread_node: &mut ThreadNode,
    current_thread_node: &mut *mut ThreadNode,
    current_thread_ptr: *mut Arc<Thread>,
    current_status: CPUStatus,
) -> (NonNull<CPUStatus>, ContextPriority, bool) {
    unsafe {
        let current_thread = &*current_thread_ptr;
        let current_tid = current_thread.tid();
        let current_context = current_thread
            .context()
            .expect("context is None before the thread is removed");
        let current_process = current_thread.process();
        let current_pid = current_process.pid();

        current_context.set_cpu_status(current_status);

        let mut status = current_thread.status_mut();
        if current_thread.is_dead() {
            *status = ContextStatus::Blocked(thread::BlockedReason::BlockedForever);
        } else if status.is_running() {
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
                let thread_tid = thread.tid();
                let process = thread.process();

                let process_pid = process.pid();
                let address_space_changed = process_pid != current_pid;

                if thread.is_dead() {
                    debug_assert!(!thread.is_removed());

                    // same tid, same thread, another thread must be the one to mark removal
                    if !address_space_changed && thread_tid == current_tid {
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
        let (cpu_status, priority, address_space_changed) = switch_inner(
            &mut **root_thread_node,
            current_thread_node_ptr,
            local.current_thread.get(),
            context,
        );
        *local.time_slices_left.get() = priority.timeslices();

        Some((cpu_status, address_space_changed))
    }
}

/// TODO: use
struct Scheduler;

/// inits the scheduler
pub unsafe fn init(main_function: fn() -> !, idle_function: fn() -> !, name: &str) -> ! {
    debug!(Scheduler, "initing ...");
    without_interrupts(|| {
        let page_table = unsafe { PhysPageTable::from_current() };
        let cwd = Box::new(make_path!("ram", "").into_owned().unwrap());

        let pid = process_list::add_pid();
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
            self::add_process(process, root_thread, None);
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

/// Appends a thread to the end of a Scheduler's threads list
/// returns the tid of the added thread
///
/// by default (if `cpu` is None) chooses the least full CPU to append to otherwise if CPU is Some(i) and i is a valid CPU index, chooses that CPU
/// use Some(0) to append to the boot CPU
pub fn add_thread(thread: Arc<Thread>, cpu: Option<usize>) {
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

    let cid = thread.tid();
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

/// Adds a combination of a process and its root thread to the scheduler.
///
/// `custom_cpu` is an optional parameter that specifies the CPU to which the thread should be assigned.
/// If `custom_cpu` is `None`, the thread will be assigned to the least loaded CPU.
pub fn add_process(process: Arc<Process>, root_thread: Arc<Thread>, custom_cpu: Option<usize>) {
    process_list::add_process(process);
    add_thread(root_thread, custom_cpu);
}
