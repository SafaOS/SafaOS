#[cfg(test)]
mod tests;

use core::cell::UnsafeCell;
use core::hint::likely;
use core::num::NonZero;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::memory::vmm::VirtualMemoryManager;
use crate::percpu::{CpuID, CpuLocal};
use crate::smp::{self, INIT_PROCESS};
use crate::thread::{
    ArcThread, BlockedReason, ContextPriority, ContextStatus, Thread, ThreadList, Tid,
};
use crate::timer::time_since_boot_ms;
use crate::utils::path::make_path;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::arch::{with_interrupts, without_interrupts};
use crate::process::Process;
use crate::utils::locks::{Mutex, TrackedSpinLock, TrackedSpinLockGuard};
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

pub fn cleanup_thread(tid: Tid, _: &'static ()) -> ! {
    crate::info!("Clean-up thread: {tid}");
    let mut cleanup_vec = ThreadList::new_empty();

    with_interrupts(|| {
        loop {
            without_interrupts(|| {
                for cpu in CpuLocal::get_all() {
                    let schd = SCHEDULER.borrow_for(cpu);
                    // Ensures post_swtch_cleanup was called before doing anything so that we know everything is in sync.
                    let _queues = schd.ready_queues.lock();

                    let mut waiting_cleanup = schd.awaiting_cleanup.lock();
                    cleanup_vec.append(&mut *waiting_cleanup);
                }
            });

            let len = cleanup_vec.len();
            for _ in 0..len {
                if let Some(thread) = cleanup_vec.pop_front() {
                    let clean_up = unsafe { thread.try_cleanup() };
                    if !clean_up {
                        unsafe { cleanup_vec.push_back(thread) };
                    }
                }
            }

            // FIXME: Block until we get a new thread?
            crate::thread::current::sleep_for_ms(50).expect("Failed to thread sleep");
        }
    })
}

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
    pub ready_queues: TrackedSpinLock<[ThreadList; PRIORITIES_COUNT]>,
    idle_thread: ArcThread,
    current_thread: UnsafeCell<ArcThread>,
    /// The head thread is the thread that is the head of the thread queue
    // pub head_thread: SpinLock<ArcThread>,
    threads_count: AtomicUsize,

    context_switch_count: AtomicUsize,
    preemption_disabled: UnsafeCell<bool>,
    stack: UnsafeCell<[u8; 1024]>,
}
impl Scheduler {
    #[inline(always)]
    /// Returns the trampoline's stack end for this scheduler.
    fn stack_end(&self) -> VirtAddr {
        VirtAddr::from_ptr(self.stack.get().wrapping_offset(1))
    }

    /// The Scheduler's IDLE loop
    pub fn idle(&'static self) -> ! {
        let cycles_per_ns = crate::arch::utils::cpu_timer_freq_mhz()
            .get()
            .div_ceil(1000);

        let cycles_per_500ns = cycles_per_ns * 500;
        crate::serial!("cycles per 500ns are: {cycles_per_500ns}\n");

        with_interrupts(|| {
            // Unfortunatlly we need interrupts so that x86 TLB invalidation works
            // The IDLE thread is guaranteed to run on this scheduler.
            loop {
                without_interrupts(|| {
                    if self.try_pop_waiting_thread() || self.try_escape_idle() {
                        crate::thread::current::yield_now();
                    }
                });

                // nano-sleep for 500ns
                let now = crate::arch::utils::cpu_cycles();
                let wait_for = now + cycles_per_500ns;
                while crate::arch::utils::cpu_cycles() < wait_for {
                    core::hint::spin_loop();
                }
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
        &'static self,
        queues: &mut [ThreadList],
        thread: ArcThread,
        index: usize,
        front: bool,
    ) {
        let head = &mut queues[index];
        unsafe { thread.set_scheduler(self) };

        if front {
            unsafe { head.push_front(thread) };
        } else {
            unsafe { head.push_back(thread) };
        }
    }

    fn add_single_thread(&'static self, queues: &mut [ThreadList], thread: ArcThread) {
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
    pub fn try_pop_waiting_thread(&'static self) -> bool {
        let mut popped = false;
        let mut waiting_threads = self.waiting_threads.lock();
        if waiting_threads.is_empty() {
            return false;
        }

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
        &'static self,
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
    pub unsafe fn schedule_thread(&'static self, thread: ArcThread, reason: ThreadScheduleReason) {
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
    fn try_giveup_thread(
        &'static self,
        f: impl FnOnce(&ArcThread) -> bool,
    ) -> Option<(ArcThread, usize)> {
        let mut ready_queues = self.ready_queues.try_lock()?;
        self.get_next_thread(&mut *ready_queues)
            .and_then(|(thread, priority)| {
                if f(&thread) {
                    self.sub_thread_count();
                    Some((thread, priority))
                } else {
                    self.add_single_thread_to(&mut *ready_queues, thread, priority, false);
                    None
                }
            })
    }

    #[inline]
    /// Tries to steal a thread from another scheduler.
    ///
    /// Returning the thread and its priority.
    fn try_steal_thread(&'static self) -> Option<(ArcThread, usize)> {
        let schedulers = CpuLocal::get_all()
            .map(|cpu| SCHEDULER.borrow_for(cpu))
            .filter(|s| !core::ptr::eq(self, *s));

        for scheduler in schedulers {
            if let Some((thread, priority)) =
                scheduler.try_giveup_thread(|thread| thread.try_set_scheduler(self))
            {
                self.threads_count.fetch_add(1, Ordering::Relaxed);
                return Some((thread, priority));
            }
        }
        None
    }

    fn try_escape_idle(&'static self) -> bool {
        let mut queues = self.ready_queues.lock();
        if queues.iter().any(|q| !q.is_empty()) {
            return true;
        }

        // Queue is empty, try to steal a thread from another scheduler
        self.try_steal_thread()
            .map(|(t, p)| {
                self.add_single_thread_to(&mut *queues, t, p, false);
            })
            .is_some()
    }

    /// Try to swap the given CPU context with the next thread's context effectively doing a context switch / thread yield.
    ///
    /// # Arguments
    /// * `current_cpu_status` - The current CPU status.
    /// * `swap_if_has_time` - Whether to swap contexts if the current thread has time left.
    ///
    /// # Returns
    /// * `Some((NonNull<CPUStatus>, bool, TrackedSpinLockGuard<[ThreadList; PRIORITIES_COUNT]>))` - The new CPU status, whether the swap was successful, and the locked schedule queues, the schedule queues shall be dropped once the kernel thread stack is swapped.
    /// * `None` - If no swap was performed.
    #[inline]
    fn try_swap_contexts(
        &'static self,
        current_cpu_status: &CPUStatus,
        swap_if_has_time: bool,
    ) -> Option<(
        NonNull<CPUStatus>,
        bool,
        TrackedSpinLockGuard<'static, [ThreadList; PRIORITIES_COUNT]>,
    )> {
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
            if !swap_if_has_time && curr_schd.has_time() {
                return None;
            }
            Some(curr_schd.get_next_priority_queue())
        } else {
            None
        };

        let mut schd_queues = self.ready_queues.lock();

        self.try_wake_waiting_threads(&mut *schd_queues, time_now, Some(&current_thread));
        self.try_boost_threads(&mut *schd_queues, time_now);

        let current_context = unsafe { current_thread.context_unchecked() };
        current_context.set_cpu_status(*current_cpu_status);
        let mut current_status = current_thread.status_mut();

        // We want to schedule the IDLE thread if the current thread is terminating so that it can be cleaned up.

        let results = self
            .get_next_thread(&mut *schd_queues)
            .or_else(|| self.try_steal_thread());

        // If there are no threads at all we schedule the IDLE thread.
        let next_thread_idle = results.is_none();

        match &*current_status {
            ContextStatus::Runnable | ContextStatus::Running => {
                if unsafe { current_context.cpu_status().as_ref() }
                    .at()
                    .is_in_lower_half()
                    && current_thread.should_terminate()
                {
                    *current_status = ContextStatus::Blocked(BlockedReason::Dead);
                    drop(current_status);
                    unsafe {
                        self.schedule_thread(current_thread.clone(), ThreadScheduleReason::Cleanup)
                    };
                } else {
                    *current_status = ContextStatus::Runnable;
                    if let Some(push_to) = push_to {
                        self.add_single_thread_to(
                            &mut *schd_queues,
                            current_thread.clone(),
                            push_to as usize,
                            false,
                        );
                    }

                    drop(current_status);
                }
            }
            ContextStatus::Blocked(BlockedReason::Dead) => unreachable!(),
            ContextStatus::Blocked(_) => {
                drop(current_status);
            }
            // The reason why we do that is to prevent anyone to wake up the thread before this causing it to be double scheduled.
            // Do nothing, its going to add itself once it is unblocked
            ContextStatus::Blocking(r) => {
                let r = *r;

                *current_status = ContextStatus::Blocked(r);
                drop(current_status);

                if r == BlockedReason::Dead {
                    unsafe {
                        self.schedule_thread(current_thread.clone(), ThreadScheduleReason::Cleanup)
                    };
                }
            }
        }

        let (new_thread, queue_index) = results.unwrap_or_else(|| (self.idle_thread.clone(), 0));

        new_thread.set_status(ContextStatus::Running);

        let schd = unsafe { &mut *new_thread.schedule_priority.get() };
        schd.queue_index = queue_index as u32;
        schd.last_scheduled = NonZero::new(time_since_boot_ms());

        let process_pid = new_thread.process().pid();
        let address_space_changed = curr_pid != process_pid;

        debug_assert!(
            address_space_changed || new_thread.tid() != current_thread.tid() || next_thread_idle,
            "Thread ID is equal, id: {}",
            new_thread.tid()
        );

        unsafe {
            let context = new_thread.context_unchecked();

            let cpu_status = context.cpu_status();
            *self.current_thread.get() = new_thread;
            Some((cpu_status, address_space_changed, schd_queues))
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
            context_switch_count: AtomicUsize::new(0),
            preemption_disabled: UnsafeCell::new(false),
            stack: UnsafeCell::new([0; 1024]),
        }
    }

    #[inline(always)]
    /// Returns the idle thread in this scheduler
    pub fn idle_thread(&self) -> &Thread {
        &self.idle_thread
    }

    #[inline(always)]
    /// Get a reference to the current thread
    /// # Safety:
    /// this reference shall not be given to other threads.
    pub unsafe fn current_thread_ref(&self) -> &ArcThread {
        unsafe { &*self.current_thread.get() }
    }

    #[inline(always)]
    /// Subtracts 1 from the thread count
    /// returns the old thread count
    fn sub_thread_count(&self) -> usize {
        self.threads_count.fetch_sub(1, Ordering::Relaxed)
    }
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

#[inline]
// used in x86_64
#[allow(unused)]
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

#[unsafe(no_mangle)]
extern "C" fn post_swtch_cleanup(schd: &'static Scheduler) {
    debug_assert!(schd.ready_queues.is_locked());
    unsafe { schd.ready_queues.force_unlock() };
}

#[inline(always)]
/// performs a context switch using the scheduler, switching to the next process context
/// to be used
/// returns the new context and a boolean indicating if the address space has changed
/// if the address space has changed, please copy the context to somewhere accessible first
///
/// returns None if the scheduler is not yet initialized or nothing is supposed to be switched to
pub fn swtch(
    context: &CPUStatus,
    before_switch: impl FnOnce(),
    is_thread_yielding: bool,
) -> Result<!, impl FnOnce()> {
    let Some(scheduler) = SCHEDULER.maybe_borrow() else {
        return Err(before_switch);
    };
    if unsafe { *scheduler.preemption_disabled.get() } {
        return Err(before_switch);
    }

    scheduler
        .context_switch_count
        .fetch_add(1, Ordering::Release);

    let Some((new_context, address_space_changed, guard)) =
        scheduler.try_swap_contexts(context, is_thread_yielding)
    else {
        return Err(before_switch);
    };

    core::mem::forget(guard);

    before_switch();
    let kernel_stack = scheduler.stack_end();
    unsafe {
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!(
            "
            mov fp, 0
            sub x22, x22, #16
            and sp, x22, #-16

            // Uses [0] as an argument to the function
            bl post_swtch_cleanup

            // Set new context as the first argument
            mov x0, x20
            tbz x21, #0, 2f
            b restore_cpu_status
            2:
                b restore_cpu_status_partial
            udf 0xC000
            "
            ,
           in("x0") scheduler,
           in("x20") new_context.as_ptr(),
           in("x21") address_space_changed as usize,
           in("x22") kernel_stack.into_raw(),
           options(noreturn)
        );

        #[cfg(target_arch = "x86_64")]
        core::arch::asm!(
            "
            mov rsp, r12
            mov rbp, rsp
            sub rsp, 64
            and rsp, -16

            // Uses [0] as an argument to the function
            call post_swtch_cleanup

            // Set new context as the first argument
            mov rdi, r14
            test r15, r15
            jz restore_cpu_status_partial_all
            jmp restore_cpu_status_full_all
            ud2
            "
            ,
           in("rdi") scheduler,
           in("r14") new_context.as_ptr(),
           in("r15") address_space_changed as usize,
           in("r12") kernel_stack.into_raw(),
           options(noreturn)
        );

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        compile_error!("Please implement the scheduler for this architecture")
    }
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
            Arc::new(VirtualMemoryManager::new_user(page_table.frame_ptr())),
            page_table,
            None,
            None,
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
