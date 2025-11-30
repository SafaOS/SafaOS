#[cfg(test)]
mod tests;

use core::cell::{SyncUnsafeCell, UnsafeCell};
use core::hint::likely;
use core::num::NonZero;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::arch::threading::CPULocal;
use crate::scheduler::wait_queue::{PendingWait, WaitQueue};
use crate::thread::{ArcThread, ContextPriority, ContextStatus, ThreadList};
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

const MIN_PRIORITY: u8 = 0;
const MAX_PRIORITY: u8 = 4;
const PRIORITIES_COUNT: usize = (MAX_PRIORITY - MIN_PRIORITY) as usize + 1;

const TIME_PER_QUANTUM: u32 = 5;
/// NOTE: Each quantum is equal to 5ms
const INITIAL_QUANTUM: u32 = 2;
/// NOTE: Each quantum is equal to 5ms so this is a multiple of 5ms.
const QUANTUM_INCREMENT: u32 = 1;

const PRIORITY_BOOST_QUANTUM: u32 = 100;
const PRIORITY_BOOST_TIME: u32 = PRIORITY_BOOST_QUANTUM * TIME_PER_QUANTUM;

#[derive(Debug, Clone, Copy)]
pub struct SchedulePriority {
    queue_index: u32,
    time_used: Option<NonZero<u64>>,
    last_scheduled: Option<NonZero<u64>>,
}

impl SchedulePriority {
    pub const fn new() -> Self {
        Self {
            queue_index: 0,
            time_used: None,
            last_scheduled: None,
        }
    }

    pub fn has_time(&self) -> bool {
        self.time_used.is_none_or(|t| {
            let time_for_queue = ((self.queue_index as u32 * QUANTUM_INCREMENT) + INITIAL_QUANTUM)
                * TIME_PER_QUANTUM;
            t.get() < time_for_queue as u64
        })
    }

    pub fn get_next_priority_queue(&mut self) -> u32 {
        if !self.has_time() {
            self.time_used = None;
            self.queue_index = (self.queue_index + 1).min(MAX_PRIORITY as u32);
        }
        self.queue_index
    }

    #[inline]
    pub fn update_time(&mut self, time_ms: NonZero<u64>) {
        let old_last_scheduled = self.last_scheduled.replace(time_ms);

        if let Some(last_scheduled) = old_last_scheduled {
            let Some(diff) = NonZero::new(time_ms.get() - last_scheduled.get()) else {
                return;
            };

            self.time_used = Some(
                self.time_used
                    .map(|u| u.saturating_add(diff.get()))
                    .unwrap_or(diff),
            );
        }
    }
}

/// The reason for scheduling a thread, within the scheduler.
///
/// using [`Scheduler::schedule_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadScheduleReason {
    /// Thread should be scheduled for sleep until the given time.
    SleepUntil(NonZero<u64>),
    /// Sleeping Thread should wake up and be scheduled for execution, that were previously scheduled with [`ThreadScheduleReason::SleepUntil`].
    UnblockTimeoutOperation,
    /// Sleeping Thread should wake up and be scheduled for execution.
    Unblocked,
    /// Thread is newly created, and we want to schedule it for execution for the first time.
    NewThread,
    /// Thread is scheduled for clean up, and it belonged lastly to this scheduler.
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerHelperSleepReason {
    WaitingToDoCleaning,
}

#[derive(Debug)]
pub struct Scheduler {
    next_wake_time: AtomicU64,
    waiting_threads: Mutex<Vec<(ArcThread, NonZero<u64>)>>,
    awaiting_cleanup: Mutex<Vec<ArcThread>>,
    helper_threads: Mutex<WaitQueue<0, SchedulerHelperSleepReason>>,
    ready_queues: SpinLock<[ThreadList; PRIORITIES_COUNT]>,
    idle_thread: ArcThread,
    pub current_thread: UnsafeCell<ArcThread>,
    /// The head thread is the thread that is the head of the thread queue
    // pub head_thread: SpinLock<ArcThread>,
    threads_count: AtomicUsize,

    is_thread_yielding: UnsafeCell<bool>,
    context_switch_count: AtomicUsize,
}
impl Scheduler {
    /// Cleanup all threads waiting for cleanup in the scheduler, and waits for new threads to be added.
    ///
    /// this is done by [`eve::thread_reaper_thread`].
    pub fn cleanup_all_and_wait(&self) {
        let mut cleanup_threads = self.awaiting_cleanup.lock();
        for thread in cleanup_threads.drain(..) {
            unsafe { thread.cleanup() };
        }
        let pending_wait = self.helper_prepare_wait();
        drop(cleanup_threads);
        pending_wait
            .enter_wait(SchedulerHelperSleepReason::WaitingToDoCleaning, None)
            .expect("Failed to wait for new cleanup threads to be added")
    }

    /// Prepares a scheduler helper thread to wait for a reason.
    pub fn helper_prepare_wait<'a>(&'a self) -> PendingWait<'a, 0, SchedulerHelperSleepReason> {
        self.helper_threads.prepare_wait()
    }

    #[inline]
    /// Boost the priority of all threads in the scheduler. if the time has come.
    fn try_boost_threads(&self, schd_queues: &mut [ThreadList], time_ms: NonZero<u64>) {
        if time_ms.get().is_multiple_of(PRIORITY_BOOST_TIME as u64) {
            let (top_queue, rest_queues) = schd_queues
                .split_first_mut()
                .expect("there should be at least one Schedule queue");

            for queue in rest_queues {
                top_queue.append(queue);
            }
        }
    }

    #[inline]
    fn get_next_thread(&self, schd_queues: &mut [ThreadList]) -> Option<(ArcThread, usize)> {
        for (index, queue) in schd_queues.iter_mut().enumerate() {
            if let Some(thread) = queue.pop_front() {
                return Some((thread, index));
            }
        }
        None
    }

    #[inline]
    fn add_single_thread_to(
        &self,
        queues: &mut [ThreadList],
        thread: ArcThread,
        index: usize,
        front: bool,
    ) {
        let head = &mut queues[index];
        unsafe {
            *thread.scheduler.get() = Some(NonNull::from_ref(self));
        }

        if front {
            unsafe { head.push_front(thread) };
        } else {
            unsafe { head.push_back(thread) };
        }
    }

    fn add_single_thread(&self, queues: &mut [ThreadList], thread: ArcThread) {
        let priority = thread.priority();
        let add_front = priority == ContextPriority::Immediate;

        let schd_pri = unsafe { &*thread.schedule_priority.get() };
        let queue = if add_front {
            0
        } else {
            schd_pri.queue_index as usize
        };

        self.add_single_thread_to(queues, thread, queue, add_front)
    }

    #[inline]
    fn try_wake_waiting_threads(&self, queues: &mut [ThreadList], time_now: NonZero<u64>) {
        const MAX_TIME: NonZero<u64> = NonZero::new(u64::MAX).expect("Is zero??????!??");

        if likely(self.next_wake_time.load(Ordering::Relaxed) > time_now.get()) {
            return;
        }

        if let Some(mut waiting_threads) = self.waiting_threads.try_lock() {
            let mut next_add_time = MAX_TIME;

            let mut i = 0;
            while i < waiting_threads.len() {
                let (_, time) = waiting_threads[i];

                if time.get() <= time_now.get() {
                    let (thread, _) = waiting_threads.swap_remove(i);
                    unsafe { thread.before_sleep_wakeup() };
                    // Same index
                    self.add_single_thread(queues, thread);
                } else {
                    next_add_time = next_add_time.min(time);
                    i += 1;
                }
            }

            self.next_wake_time
                .store(next_add_time.get(), Ordering::Relaxed);
        }
    }

    /// Schedules a thread for execution on this scheduler.
    ///
    /// Safe to call from any context as long as the thread doesn't belong to any scheduler, should never deadlock.
    pub unsafe fn schedule_thread(&self, thread: ArcThread, reason: ThreadScheduleReason) {
        if reason == ThreadScheduleReason::UnblockTimeoutOperation {
            let mut waiting_threads = self.waiting_threads.lock();
            if let Some(index) = waiting_threads.iter().position(|(t, _)| *t == thread) {
                waiting_threads.swap_remove(index);
            } else {
                // Already unblocked
                return;
            }
        }

        match reason {
            ThreadScheduleReason::SleepUntil(time) => {
                let mut waiting_threads = self.waiting_threads.lock();
                waiting_threads.push((thread, time));
                self.next_wake_time.fetch_min(time.get(), Ordering::Relaxed);
            }
            ThreadScheduleReason::Cleanup => {
                let mut awaiting_cleanup = self.awaiting_cleanup.lock();
                awaiting_cleanup.push(thread);
                self.helper_threads
                    .lock()
                    .wake_equals(&SchedulerHelperSleepReason::WaitingToDoCleaning);
            }
            _ => {
                // Cannot risk deadlocking the current scheduler
                without_interrupts(|| {
                    let mut queues = self.ready_queues.lock();
                    self.add_single_thread(&mut *queues, thread);
                });

                if reason == ThreadScheduleReason::NewThread {
                    self.threads_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    #[inline]
    fn try_yield_execution(
        &self,
        current_cpu_status: CPUStatus,
        yield_if_has_time: bool,
    ) -> Option<(NonNull<CPUStatus>, ContextPriority, bool)> {
        let current_thread = unsafe { &mut *self.current_thread.get() };
        let curr_pid = current_thread.process().pid();

        let time_now = crate::time!(ms);
        let Some(time_now) = NonZero::new(time_now) else {
            return None;
        };

        let curr_schd = unsafe { &mut *current_thread.schedule_priority.get() };
        curr_schd.update_time(time_now);
        if !yield_if_has_time && curr_schd.has_time() {
            return None;
        }

        let mut schd_queues = self.ready_queues.lock();

        self.try_wake_waiting_threads(&mut *schd_queues, time_now);
        self.try_boost_threads(&mut *schd_queues, time_now);

        // Reschedule the current thread if no threads, meaning we are the IDLE thread
        let (new_thread, queue_index) = self
            .get_next_thread(&mut *schd_queues)
            .unwrap_or_else(|| (self.idle_thread.clone(), 0));

        let push_to = curr_schd.get_next_priority_queue();
        if likely(!current_thread.is_dead()) {
            let current_context = unsafe { current_thread.context_unchecked() };
            current_context.set_cpu_status(current_cpu_status);

            let mut current_status = current_thread.status_mut();
            match &*current_status {
                ContextStatus::Runnable | ContextStatus::Running => {
                    *current_status = ContextStatus::Runnable;
                    drop(current_status);
                    self.add_single_thread_to(
                        &mut *schd_queues,
                        current_thread.clone(),
                        push_to as usize,
                        false,
                    );
                }
                ContextStatus::Blocked(_) => {}
                // The reason why we do that is to prevent anyone to wake up the thread before this causing it to be double scheduled.
                // Do nothing, its going to add itself once it is unblocked
                ContextStatus::Blocking(r) => *current_status = ContextStatus::Blocked(*r),
            }
        }

        if self.idle_thread != new_thread {
            new_thread.set_status(ContextStatus::Running);
        } else {
            // Once its rescheduled, it will be blocked.
            new_thread.set_status(ContextStatus::Blocking(
                crate::thread::BlockedReason::Waiting,
            ));
        }

        let schd = unsafe { &mut *new_thread.schedule_priority.get() };
        schd.queue_index = queue_index as u32;
        schd.last_scheduled = NonZero::new(crate::time!(ms));

        let context_priority = new_thread.priority();
        let process_pid = new_thread.process().pid();
        let address_space_changed = curr_pid != process_pid;

        debug_assert!(
            address_space_changed
                || new_thread.tid() != current_thread.tid()
                || self.idle_thread == new_thread,
            "Thread ID is equal, id: {}",
            new_thread.tid()
        );
        unsafe {
            let context = new_thread.context_unchecked();

            let cpu_status = context.cpu_status();
            *self.current_thread.get() = new_thread;
            Some((cpu_status, context_priority, address_space_changed))
        }
    }

    pub fn new_in_cpu(idle_thread: ArcThread) -> &'static mut CPULocal {
        let r = CPULocal::allocate_with_scheduler(Self {
            idle_thread: idle_thread.clone(),
            waiting_threads: Mutex::new(Vec::new()),
            awaiting_cleanup: Mutex::new(Vec::new()),
            next_wake_time: AtomicU64::new(u64::MAX),
            ready_queues: SpinLock::new(core::array::from_fn(|_| ThreadList::new_empty())),
            current_thread: UnsafeCell::new(idle_thread.clone()),
            threads_count: AtomicUsize::new(0),
            is_thread_yielding: UnsafeCell::new(false),
            context_switch_count: AtomicUsize::new(0),
            helper_threads: Mutex::new(WaitQueue::new()),
        });

        unsafe {
            *idle_thread.scheduler.get() = Some(NonNull::from_ref(r.scheduler()));
            // The next time it is scheduled to run its going to be blocked.
            idle_thread.block_waiting();
        }
        r
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
        self.threads_count.fetch_sub(1, Ordering::Relaxed)
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

/// Returns the number of CPUs.
pub fn cpu_count() -> usize {
    Scheduler::get_all().len()
}

pub static SCHEDULER_INITED: SyncUnsafeCell<bool> = SyncUnsafeCell::new(false);

/// Scheduler should be initialized first
pub(super) unsafe fn before_thread_yield() {
    unsafe {
        *Scheduler::get().is_thread_yielding.get() = true;
    }
}

// #[inline]
/// context switches into next process, takes current context outputs new context
/// returns the new context and a boolean indicating if the address space has changed
// unsafe fn switch_inner(
//     head_thread: &ArcThread,
//     current_thread_ptr: *mut ArcThread,
//     current_cpu_status: CPUStatus,
// ) -> (NonNull<CPUStatus>, ContextPriority, bool) {
//     unsafe {
//         let current_thread = &*current_thread_ptr;
//         let current_process = current_thread.process();
//         let current_pid = current_process.pid();

//         if likely(!current_thread.is_dead()) {
//             let mut status = current_thread.status_mut();

//             let current_context = current_thread.context_unchecked();
//             current_context.set_cpu_status(current_cpu_status);

//             if *status == ContextStatus::Running {
//                 *status = ContextStatus::Runnable;
//             }
//         }

//         let try_choose_thread = |choose: &ArcThread| {
//             assert!(!choose.is_dead());

//             let process = choose.process();
//             let process_pid = process.pid();
//             let address_space_changed = process_pid != current_pid;

//             let mut status = choose.status_mut();

//             macro_rules! choose_context {
//                 () => {{
//                     *status = ContextStatus::Running;
//                     let priority = choose.priority();

//                     let context = choose.context_unchecked();
//                     let cpu_status = context.cpu_status();
//                     drop(status);
//                     *current_thread_ptr = choose.clone();
//                     Some((cpu_status, priority, address_space_changed))
//                 }};
//             }

//             match &*status {
//                 ContextStatus::Runnable => return choose_context!(),
//                 ContextStatus::Blocked(_) => None,
//                 ContextStatus::Running => unreachable!(),
//             }
//         };

//         let mut current = current_thread.next().as_ref().unwrap_or_else(|| {
//             try_wake_sleeping();
//             head_thread
//         });
//         loop {
//             if let Some(results) = try_choose_thread(current) {
//                 return results;
//             }

//             current = match current.next().as_ref() {
//                 Some(s) => s,
//                 None => {
//                     try_wake_sleeping();
//                     head_thread
//                 }
//             };
//         }
//     }
// }

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

    let scheduler = Scheduler::get();
    let is_thread_yielding = unsafe { scheduler.is_thread_yielding.get().replace(false) };

    let (cpu_status, _, address_space_changed) =
        scheduler.try_yield_execution(context, is_thread_yielding)?;

    scheduler
        .context_switch_count
        .fetch_add(1, Ordering::Release);
    Some((cpu_status, address_space_changed))
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
            .min_by_key(|(_, scheduler)| scheduler.threads_count.load(Ordering::Relaxed))
            .expect("no CPU found")
    };

    let cid = thread.tid();
    let pid = thread.process().pid();

    unsafe { scheduler.schedule_thread(thread, ThreadScheduleReason::NewThread) };
    scheduler.threads_count.fetch_add(1, Ordering::Relaxed);

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
