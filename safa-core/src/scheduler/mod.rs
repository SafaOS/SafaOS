#[cfg(test)]
mod tests;

use core::cell::UnsafeCell;
use core::hint::likely;
use core::num::NonZero;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::percpu::{CpuID, CpuLocal};
use crate::smp::{self, INIT_PROCESS};
use crate::thread::{ArcThread, BlockedReason, ContextPriority, ContextStatus, Thread, ThreadList};
use crate::timer::time_since_boot_ms;
use crate::utils::path::make_path;
use alloc::collections::vec_deque::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::arch::{with_interrupts, without_interrupts};
use crate::process::Process;
use crate::utils::locks::{Mutex, TrackedSpinLock};
use crate::utils::types::Name;
use crate::{VirtAddr, eve, percpu};
use alloc::boxed::Box;

pub mod process_list;
pub mod wait_queue;

use crate::{
    arch::threading::{CPUStatus, restore_cpu_status},
    debug,
    memory::paging::PhysPageTable,
};

percpu::define! {
    /// The Scheduler for the current CPU.
    pub static SCHEDULER: Scheduler = {
        let process = unsafe {(*INIT_PROCESS.get()).as_ref().expect("Running scheduler's initializer before the init processes has been decided")};
        let (idle_thread, _) = process
            .threads_manager()
            .create_thread(
                process,
                VirtAddr::from(eve::idle_function as usize),
                VirtAddr::null(),
                Some(ContextPriority::Low),
                None,
            )
            .expect("Failed to create the idle thread for a CPU");

        Scheduler::new(idle_thread)
    };
}

const MIN_PRIORITY: u8 = 0;
const MAX_PRIORITY: u8 = 4;
const PRIORITIES_COUNT: usize = (MAX_PRIORITY - MIN_PRIORITY) as usize + 1;

pub const TIME_PER_QUANTUM: u32 = 3;
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

#[derive(Debug)]
pub struct Scheduler {
    next_wake_time: AtomicU64,
    waiting_threads: Mutex<Vec<(ArcThread, NonZero<u64>)>>,
    awaiting_cleanup: TrackedSpinLock<ThreadList>,
    ready_queues: TrackedSpinLock<[ThreadList; PRIORITIES_COUNT]>,
    idle_thread: ArcThread,
    current_thread: UnsafeCell<ArcThread>,
    /// The head thread is the thread that is the head of the thread queue
    // pub head_thread: SpinLock<ArcThread>,
    threads_count: AtomicUsize,

    is_thread_yielding: UnsafeCell<bool>,
    context_switch_count: AtomicUsize,
    is_idle: UnsafeCell<bool>,
    preemption_disabled: UnsafeCell<bool>,
}
impl Scheduler {
    /// The Scheduler's IDLE loop
    pub fn idle(&self) -> ! {
        // My fingers were guided to pick 6 here randomly, it stays that way...
        let mut cleanup_vec = VecDeque::with_capacity(6);

        with_interrupts(|| {
            // Unfortunatlly we need interrupts so that x86 TLB invalidation works
            // The IDLE thread is guaranteed to run on this scheduler.
            loop {
                let should_yield = without_preemption(|| {
                    let mut waiting_cleanup = self.awaiting_cleanup.lock();
                    while let Some(thread) = waiting_cleanup.pop_front() {
                        // Avoids anything from the cleanup-routine causing deadlocks because the lock wasn't dropped.
                        //
                        // FIXME: This shouldn't be a problem.
                        cleanup_vec.push_back(thread);
                    }
                    drop(waiting_cleanup);

                    let len = cleanup_vec.len();
                    for _ in 0..len {
                        if let Some(thread) = cleanup_vec.pop_front() {
                            // FIXME: Some kind of a hidden Drop impl may thread yield here, so I had to come up with this temporarily.
                            if !unsafe { thread.try_cleanup() } {
                                // TODO: ??
                                cleanup_vec.push_back(thread);
                            }
                        }
                    }

                    !self.is_idle() || self.try_pop_waiting_thread()
                });

                if should_yield {
                    // Give up the CPU to the next thread
                    crate::thread::current::yield_now();
                }

                core::hint::spin_loop();
            }
        })
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
            thread.set_scheduler(self);
        }

        if front {
            unsafe { head.push_front(thread) };
        } else {
            unsafe { head.push_back(thread) };
        }

        unsafe {
            *self.is_idle.get() = false;
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
    pub fn try_pop_waiting_thread(&self) -> bool {
        let mut popped = false;
        let mut waiting_threads = self.waiting_threads.lock();
        let time_now = time_since_boot_ms();

        let mut next_add_time: NonZero<u64> = NonZero::<u64>::MAX;

        let mut i = 0;
        while i < waiting_threads.len() {
            let (_, time) = &waiting_threads[i];

            if time.get() <= time_now {
                let (thread, _) = waiting_threads.swap_remove(i);
                unsafe { thread.before_sleep_wakeup() };
                unsafe { self.schedule_thread(thread, ThreadScheduleReason::Unblocked) };
                popped = true;
            } else {
                next_add_time = next_add_time.min(*time);
                i += 1;
            }
        }

        self.next_wake_time
            .fetch_min(next_add_time.get(), Ordering::Relaxed);
        popped
    }

    #[inline]
    fn try_wake_waiting_threads(
        &self,
        queues: &mut [ThreadList],
        time_now: NonZero<u64>,
        current_thread: Option<&ArcThread>,
    ) {
        const MAX_TIME: NonZero<u64> = NonZero::new(u64::MAX).expect("Is zero??????!??");

        if likely(self.next_wake_time.load(Ordering::Relaxed) > time_now.get()) {
            return;
        }

        if let Some(mut waiting_threads) = self.waiting_threads.try_lock() {
            let mut next_add_time = MAX_TIME;

            let mut i = 0;
            while i < waiting_threads.len() {
                let (th, time) = &waiting_threads[i];

                if likely(Some(th) != current_thread) && time.get() <= time_now.get() {
                    let (thread, _) = waiting_threads.swap_remove(i);
                    unsafe { thread.before_sleep_wakeup() };
                    // Same index
                    self.add_single_thread(queues, thread);
                } else {
                    next_add_time = next_add_time.min(*time);
                    i += 1;
                }
            }

            self.next_wake_time
                .fetch_min(next_add_time.get(), Ordering::Relaxed);
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
                without_interrupts(|| {
                    let mut awaiting_cleanup = self.awaiting_cleanup.lock();
                    unsafe { awaiting_cleanup.push_back(thread) };
                });
                self.sub_thread_count();
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
    /// Tries to give up a thread to another scheduler.
    ///
    /// Returns the thread and its priority.
    fn try_giveup_thread(&self) -> Option<(ArcThread, usize)> {
        self.ready_queues.try_lock().and_then(|mut q| {
            let r = self.get_next_thread(&mut *q);
            self.sub_thread_count();
            r
        })
    }

    #[inline]
    /// Tries to steal a thread from another scheduler.
    ///
    /// Returning the thread and its priority.
    fn try_steal_thread(&self) -> Option<(ArcThread, usize)> {
        let schedulers = CpuLocal::get_all()
            .map(|cpu| SCHEDULER.borrow_for(cpu))
            .filter(|s| !core::ptr::eq(self, *s));

        for scheduler in schedulers {
            if let Some((thread, priority)) = scheduler.try_giveup_thread() {
                unsafe { thread.set_scheduler(self) };
                self.threads_count.fetch_add(1, Ordering::Relaxed);
                return Some((thread, priority));
            }
        }
        None
    }

    #[inline]
    fn try_yield_execution(
        &self,
        current_cpu_status: CPUStatus,
        yield_if_has_time: bool,
    ) -> Option<(NonNull<CPUStatus>, ContextPriority, bool)> {
        let current_thread = unsafe { &mut *self.current_thread.get() };
        let curr_pid = current_thread.process().pid();

        let time_now = time_since_boot_ms();
        let Some(time_now) = NonZero::new(time_now) else {
            return None;
        };

        let was_idle_thread = *current_thread == self.idle_thread;
        let push_to = if !was_idle_thread {
            let curr_schd = unsafe { &mut *current_thread.schedule_priority.get() };
            curr_schd.update_time(time_now);
            if !yield_if_has_time && curr_schd.has_time() {
                return None;
            }
            Some(curr_schd.get_next_priority_queue())
        } else {
            None
        };

        let mut schd_queues = self.ready_queues.lock();

        self.try_wake_waiting_threads(&mut *schd_queues, time_now, Some(&current_thread));
        self.try_boost_threads(&mut *schd_queues, time_now);

        let mut is_idle = schd_queues.iter().all(|q| q.is_empty());

        let current_context = unsafe { current_thread.context_unchecked() };
        current_context.set_cpu_status(current_cpu_status);
        let mut current_status = current_thread.status_mut();

        // We want to schedule the IDLE thread if the current thread is terminating so that it can be cleaned up.
        let is_terminating = matches!(
            &*current_status,
            ContextStatus::Blocked(BlockedReason::Dead)
                | ContextStatus::Blocking(BlockedReason::Dead)
        );

        let results = (!is_terminating)
            .then(|| {
                self.get_next_thread(&mut *schd_queues)
                    .or_else(|| self.try_steal_thread())
            })
            .flatten();

        // If there are no threads at all we schedule the IDLE thread.
        let next_thread_idle = results.is_none();

        match &*current_status {
            ContextStatus::Runnable | ContextStatus::Running => {
                *current_status = ContextStatus::Runnable;
                drop(current_status);
                if let Some(push_to) = push_to {
                    self.add_single_thread_to(
                        &mut *schd_queues,
                        current_thread.clone(),
                        push_to as usize,
                        false,
                    );

                    is_idle = false;
                }
            }
            ContextStatus::Blocked(_) => {
                drop(current_status);
            }
            // The reason why we do that is to prevent anyone to wake up the thread before this causing it to be double scheduled.
            // Do nothing, its going to add itself once it is unblocked
            ContextStatus::Blocking(r) => {
                *current_status = ContextStatus::Blocked(*r);
                drop(current_status);
            }
        }

        let (new_thread, queue_index) = results.unwrap_or_else(|| (self.idle_thread.clone(), 0));

        new_thread.set_status(ContextStatus::Running);

        let schd = unsafe { &mut *new_thread.schedule_priority.get() };
        schd.queue_index = queue_index as u32;
        schd.last_scheduled = NonZero::new(time_since_boot_ms());

        let context_priority = new_thread.priority();
        let process_pid = new_thread.process().pid();
        let address_space_changed = curr_pid != process_pid;

        debug_assert!(
            address_space_changed || new_thread.tid() != current_thread.tid() || next_thread_idle,
            "Thread ID is equal, id: {}",
            new_thread.tid()
        );

        unsafe {
            *self.is_idle.get() = is_idle;
        }
        unsafe {
            let context = new_thread.context_unchecked();

            let cpu_status = context.cpu_status();
            *self.current_thread.get() = new_thread;
            Some((cpu_status, context_priority, address_space_changed))
        }
    }

    /// Constructs a new scheduler
    ///
    /// NOTE: The idle thread is not aware of this scheduler being its parent yet...
    pub fn new(idle_thread: ArcThread) -> Self {
        // The next time it is scheduled to run its going to be blocked.
        unsafe {
            idle_thread.block_waiting();
        }
        Self {
            idle_thread: idle_thread.clone(),
            waiting_threads: Mutex::new(Vec::new()),
            awaiting_cleanup: TrackedSpinLock::new(ThreadList::new_empty()),
            next_wake_time: AtomicU64::new(u64::MAX),
            ready_queues: TrackedSpinLock::new(core::array::from_fn(|_| ThreadList::new_empty())),
            current_thread: UnsafeCell::new(idle_thread.clone()),
            threads_count: AtomicUsize::new(0),
            is_thread_yielding: UnsafeCell::new(false),
            context_switch_count: AtomicUsize::new(0),
            is_idle: UnsafeCell::new(true),
            preemption_disabled: UnsafeCell::new(false),
        }
    }

    /// Returns the idle thread in this scheduler
    pub fn idle_thread(&self) -> &Thread {
        &self.idle_thread
    }

    /// Get a reference to the current thread
    /// # Safety:
    /// this reference shall not be given to other threads.
    pub unsafe fn current_thread_ref(&self) -> &ArcThread {
        unsafe { &*self.current_thread.get() }
    }

    /// Subtracts 1 from the thread count
    /// returns the old thread count
    fn sub_thread_count(&self) -> usize {
        self.threads_count.fetch_sub(1, Ordering::Relaxed)
    }

    /// Scheduler is IDLE hint
    pub fn is_idle(&self) -> bool {
        unsafe { *self.is_idle.get() }
    }
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

#[must_use = "returns whether or not the scheduler was initialized"]
pub(super) unsafe fn before_thread_yield() -> bool {
    unsafe {
        if let Some(scheduler) = SCHEDULER.maybe_borrow() {
            *scheduler.is_thread_yielding.get() = true;
            true
        } else {
            core::hint::cold_path();
            false
        }
    }
}

#[inline]
/// Disables preemption for the duration of the closure.
pub fn without_preemption<F, R>(mut f: F) -> R
where
    F: FnMut() -> R,
{
    let mut schd = None;
    let mut preemption_disabled = false;

    without_interrupts(|| {
        schd = SCHEDULER.maybe_borrow();
        preemption_disabled = schd
            .map(|schd| unsafe { schd.preemption_disabled.get().replace(true) })
            .unwrap_or(false);
    });

    let result = f();

    if let Some(schd) = schd {
        unsafe {
            *schd.preemption_disabled.get() = preemption_disabled;
        }
    }
    result
}

#[inline(always)]
/// performs a context switch using the scheduler, switching to the next process context
/// to be used
/// returns the new context and a boolean indicating if the address space has changed
/// if the address space has changed, please copy the context to somewhere accessible first
///
/// returns None if the scheduler is not yet initialized or nothing is supposed to be switched to
pub fn swtch(context: CPUStatus) -> Option<(NonNull<CPUStatus>, bool)> {
    let scheduler = SCHEDULER.maybe_borrow()?;
    let is_thread_yielding = unsafe { scheduler.is_thread_yielding.get().replace(false) };
    if unsafe { *scheduler.preemption_disabled.get() } {
        return None;
    }

    let (cpu_status, _, address_space_changed) =
        scheduler.try_yield_execution(context, is_thread_yielding)?;

    scheduler
        .context_switch_count
        .fetch_add(1, Ordering::Release);
    Some((cpu_status, address_space_changed))
}

/// inits the scheduler
pub unsafe fn init(main_function: fn() -> !, name: &str) -> ! {
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
            smp::init_cpus(process.clone());

            let status = SCHEDULER.idle_thread().context_unchecked().cpu_status();
            let status_ref = status.as_ref();

            self::add_process(process, root_thread, None);

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
    let cpu_id = cpu
        .filter(|i| *i <= u16::MAX as usize)
        .map(|i| CpuID::from_u16(i as u16))
        .flatten();

    let chosen_cpu = cpu_id.map(|id| CpuLocal::get_for(id)).unwrap_or_else(|| {
        let cpus = CpuLocal::get_all();
        cpus.min_by_key(|cpu| {
            SCHEDULER
                .borrow_for(cpu)
                .threads_count
                .load(Ordering::Relaxed)
        })
        .expect("No CPUs available")
    });

    let chosen_schd = SCHEDULER.borrow_for(chosen_cpu);

    let tid = thread.tid();
    let pid = thread.process().pid();

    unsafe { chosen_schd.schedule_thread(thread, ThreadScheduleReason::NewThread) };

    debug!(
        Scheduler,
        "Thread {tid} added for process {pid}, CPU: {:?}", chosen_cpu.cpu_id
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
