#[cfg(test)]
mod tests;

use core::cell::{SyncUnsafeCell, UnsafeCell};
use core::hint::likely;
use core::num::NonZero;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::thread::{ArcThread, ContextPriority, ContextStatus};
use crate::utils::path::make_path;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::arch::without_interrupts;
use crate::process::Process;
use crate::utils::locks::{Mutex, SpinLock};
use crate::utils::types::Name;
use crate::{VirtAddr, arch};
use alloc::boxed::Box;

pub mod process_list;
pub mod wait_queue;

use crate::{
    arch::threading::{CPUStatus, restore_cpu_status},
    debug,
    memory::paging::PhysPageTable,
};

#[derive(Debug, Clone)]
struct SleepingThread {
    wake_at: NonZero<u64>,
    thread: ArcThread,
}

impl SleepingThread {
    #[inline(always)]
    pub const fn new(thread: ArcThread, wakeup_time: NonZero<u64>) -> Self {
        SleepingThread {
            wake_at: wakeup_time,
            thread,
        }
    }
}

struct SleepingThreads {
    threads: Vec<SleepingThread>,
    next_wake_at: Option<NonZero<u64>>,
}

impl SleepingThreads {
    pub const fn new() -> Self {
        Self {
            threads: Vec::new(),
            next_wake_at: None,
        }
    }

    #[inline]
    pub fn wake_all_matches(&mut self, mut f: impl FnMut(&SleepingThread) -> bool) {
        let mut i = 0;
        let mut new_next_wake_at = None;

        while i < self.threads.len() {
            let Some(thread) = self.threads.get(i) else {
                break;
            };

            if f(&thread) {
                // Swap remove replaces elements so the index is still there for the next iteration.
                let thread = self.threads.swap_remove(i).thread;
                // Wakes up sleeping thread...
                thread.wake_up_sleeping_for_time();
                continue;
            }

            if new_next_wake_at.is_none_or(|val| val > thread.wake_at) {
                new_next_wake_at = Some(thread.wake_at);
            }
            i += 1;
        }

        self.next_wake_at = new_next_wake_at;
    }
}

/// Interacting with this must be done by
/// - [`try_wake_sleeping`] unsafe.
/// - [`add_sleeping_thread`] safe.
/// only.
static SLEEPING_THREADS: Mutex<SleepingThreads> = Mutex::new(SleepingThreads::new());

fn try_wake_sleeping() {
    let Some(mut sleeping_threads) = SLEEPING_THREADS.try_lock() else {
        return;
    };

    let next_wake_at = sleeping_threads.next_wake_at;
    let Some(next_wake_at_val) = next_wake_at else {
        return;
    };

    let time = crate::time!(ms);
    if time >= next_wake_at_val.get() {
        sleeping_threads.wake_all_matches(|entry| entry.wake_at.get() <= time);
    }
}

/// Adds thread `thread` to the sleeping queue, so that it wake up at `wake_at`.
///
/// Returns false if the thread should wake up immediately otherwise true.
pub fn add_sleeping_thread(thread: ArcThread, wake_after_ms: NonZero<u64>) -> bool {
    let wake_at = NonZero::new(wake_after_ms.get() + crate::time!(ms)).expect("Shouldn't fail");

    let mut sleeping_threads = SLEEPING_THREADS.lock();
    if crate::time!(ms) >= wake_at.get() {
        return false;
    }

    sleeping_threads
        .threads
        .push(SleepingThread::new(thread, wake_at));

    if sleeping_threads.next_wake_at.is_none_or(|v| v > wake_at) {
        sleeping_threads.next_wake_at = Some(wake_at);
    }
    true
}

/// MUST BE CALLED If you are going to be awaking a sleeping thread early before it goes to sleep again.
pub fn remove_sleeping_matches(matches: &ArcThread) {
    let mut sleeping_threads = SLEEPING_THREADS.lock();

    if let Some(idx) = sleeping_threads
        .threads
        .iter()
        .position(|t| Arc::ptr_eq(matches, &t.thread))
    {
        sleeping_threads.threads.swap_remove(idx);
    }
}

#[derive(Debug)]
pub struct Scheduler {
    pub current_thread: UnsafeCell<ArcThread>,
    /// The head thread is the thread that is the head of the thread queue
    pub head_thread: SpinLock<ArcThread>,
    threads_count: AtomicUsize,

    time_slices_left: SyncUnsafeCell<u32>,
    context_switch_count: AtomicUsize,
}
impl Scheduler {
    pub fn new(idle_thread: ArcThread) -> Self {
        Self {
            current_thread: UnsafeCell::new(idle_thread.clone()),
            head_thread: SpinLock::new(idle_thread.clone()),
            threads_count: AtomicUsize::new(0),
            time_slices_left: SyncUnsafeCell::new(0),
            context_switch_count: AtomicUsize::new(0),
        }
    }
    /// Get a reference to the current thread
    /// # Safety:
    /// this reference shall not be given to other threads.
    pub unsafe fn current_thread_ref(&self) -> &ArcThread {
        unsafe { &*self.current_thread.get() }
    }

    /// Subtracts 1 from the thread count
    /// returns the old thread count
    pub fn sub_thread_count(&self) -> usize {
        self.threads_count.fetch_sub(1, Ordering::AcqRel)
    }

    /// Get a reference to the number of context switches that have been done
    pub fn context_switches_count_ref(&self) -> &AtomicUsize {
        &self.context_switch_count
    }
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    /// Get a reference to the current Scheduler
    pub fn get() -> &'static Self {
        unsafe { &*arch::threading::cpu_local_storage_ptr().cast() }
    }
    /// Get a reference to all Schedulers for all CPUs
    pub fn get_all() -> &'static [&'static Self] {
        unsafe { arch::threading::cpu_local_storages() }
    }
}

/// Subtracts one timeslice from the current context's timeslices passed.
/// Returns `true` if the current context has finished all of its timeslices.
unsafe fn timeslices_sub_finished() -> bool {
    let local = Scheduler::get();
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
        *Scheduler::get().time_slices_left.get() = 0;
    }
}

#[inline]
/// context switches into next process, takes current context outputs new context
/// returns the new context and a boolean indicating if the address space has changed
unsafe fn switch_inner(
    head_thread: &ArcThread,
    current_thread_ptr: *mut ArcThread,
    current_cpu_status: CPUStatus,
) -> (NonNull<CPUStatus>, ContextPriority, bool) {
    unsafe {
        let current_thread = &*current_thread_ptr;
        let current_process = current_thread.process();
        let current_pid = current_process.pid();

        if likely(!current_thread.is_dead()) {
            let mut status = current_thread.status_mut();

            let current_context = current_thread.context_unchecked();
            current_context.set_cpu_status(current_cpu_status);

            if *status == ContextStatus::Running {
                *status = ContextStatus::Runnable;
            }
        }

        let try_choose_thread = |choose: &ArcThread| {
            assert!(!choose.is_dead());

            let process = choose.process();
            let process_pid = process.pid();
            let address_space_changed = process_pid != current_pid;

            let mut status = choose.status_mut();

            macro_rules! choose_context {
                () => {{
                    *status = ContextStatus::Running;
                    let priority = choose.priority();

                    let context = choose.context_unchecked();
                    let cpu_status = context.cpu_status();
                    drop(status);
                    *current_thread_ptr = choose.clone();
                    Some((cpu_status, priority, address_space_changed))
                }};
            }

            match &*status {
                ContextStatus::Runnable => return choose_context!(),
                ContextStatus::Blocked(_) => None,
                ContextStatus::Running => unreachable!(),
            }
        };

        let mut current = current_thread.next().as_ref().unwrap_or_else(|| {
            try_wake_sleeping();
            head_thread
        });
        loop {
            if let Some(results) = try_choose_thread(current) {
                return results;
            }

            current = match current.next().as_ref() {
                Some(s) => s,
                None => {
                    try_wake_sleeping();
                    head_thread
                }
            };
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

    let scheduler = Scheduler::get();
    let head_thread = scheduler.head_thread.lock();
    scheduler
        .context_switch_count
        .fetch_add(1, Ordering::Release);

    unsafe {
        let (cpu_status, priority, address_space_changed) =
            switch_inner(&*head_thread, scheduler.current_thread.get(), context);
        *scheduler.time_slices_left.get() = priority.timeslices();

        Some((cpu_status, address_space_changed))
    }
}

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
pub fn add_thread(thread: ArcThread, cpu: Option<usize>) {
    let schedulers = Scheduler::get_all();

    let (cpu_index, scheduler) = if let Some(cpu) = cpu
        && let Some(scheduler) = schedulers.get(cpu)
    {
        (cpu, scheduler)
    } else {
        schedulers
            .iter()
            .enumerate()
            .min_by_key(|(_, scheduler)| scheduler.threads_count.load(Ordering::Acquire))
            .expect("no CPU found")
    };

    let cid = thread.tid();
    let pid = thread.process().pid();

    without_interrupts(
        /* lock scheduler without interrupts enabled so we don't lock ourself */
        move || {
            let mut head_thread = scheduler.head_thread.lock();
            unsafe {
                // FIXME: the idle thread should put the scheduler in itself
                *head_thread.scheduler.get() = NonNull::new(*scheduler as *const _ as *mut _);
                head_thread.add_to_head_thread(thread);
            }

            scheduler.threads_count.fetch_add(1, Ordering::SeqCst);
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
pub fn add_process(process: Arc<Process>, root_thread: ArcThread, custom_cpu: Option<usize>) {
    process_list::add_process(process);
    add_thread(root_thread, custom_cpu);
}
