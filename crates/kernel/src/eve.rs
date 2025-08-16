//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use core::cell::SyncUnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::drivers::driver_poll::{self, PolledDriver};
use crate::process::Process;
use crate::serial;
use crate::thread::{self, ArcThread, ContextPriority, Tid};
use crate::utils::alloc::PageString;
use crate::utils::locks::Mutex;
use crate::utils::path::make_path;
use crate::{debug, fs, logging, process};
use alloc::sync::Arc;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use safa_abi::fs::OpenOptions;
use safa_abi::process::ProcessStdio;
use spin::Lazy;

pub(super) static KERNEL_STDIO: Lazy<ProcessStdio> = Lazy::new(|| {
    let stdin =
        fs::FileRef::open_with_options(make_path!("dev", "tty"), OpenOptions::READ).unwrap();
    let stdout =
        fs::FileRef::open_with_options(make_path!("dev", "tty"), OpenOptions::WRITE).unwrap();
    let stderr = stdout.dup();
    ProcessStdio::new(Some(stdout.fd()), Some(stdin.fd()), Some(stderr.fd()))
});

lazy_static! {
    static ref POLLING: SyncUnsafeCell<Vec<&'static dyn PolledDriver>> =
        SyncUnsafeCell::new(driver_poll::take_poll());
}

pub enum CleanupItem {
    Thread {
        context_switch_count: &'static AtomicUsize,
        at_context_switch_count: usize,
        thread: ArcThread,
    },
    Process {
        proc: Arc<Process>,
    },
}

unsafe impl Send for CleanupItem {}
unsafe impl Sync for CleanupItem {}

static SHOULD_WAKEUP: AtomicUsize = AtomicUsize::new(0);
static TO_CLEANUP: Mutex<Vec<CleanupItem>> = Mutex::new(Vec::new());

fn poll_driver_thread(tid: Tid, driver: &&dyn PolledDriver) -> ! {
    debug!(
        "polling driver in thread: {}, thread TID: {tid}",
        driver.thread_name()
    );
    driver.poll_function()
}

/// the main loop of Eve
/// it will run until doomsday
pub fn main() -> ! {
    *logging::SERIAL_LOG.write() = Some(PageString::new());
    crate::info!("eve has been awaken ...");
    // TODO: make a macro or a const function to do this automatically
    serial!("Hello, world!, running tests...\n",);

    // FIXME: use threads
    for poll_driver in unsafe { &*POLLING.get() } {
        process::current::kernel_thread_spawn(
            poll_driver_thread,
            poll_driver,
            Some(ContextPriority::High),
            Some(0),
        )
        .expect("failed to spawn a thread function for a polled driver");
    }

    #[cfg(not(test))]
    {
        use crate::process::spawn::{SpawnFlags, pspawn};
        use crate::utils::types::Name;

        // start the shell
        pspawn(
            Name::try_from("Shell").unwrap(),
            // Maybe we can make a const function or a macro for this
            make_path!("sys", "bin/safa"),
            &["sys:/bin/safa", "-i"],
            &[b"PATH=sys:/bin", b"SHELL=sys:/bin/safa"],
            SpawnFlags::empty(),
            ContextPriority::Medium,
            *KERNEL_STDIO,
            None,
        )
        .unwrap();
    }

    #[cfg(test)]
    {
        use crate::thread::ContextPriority;

        fn run_tests(_tid: Tid, _arg: &()) -> ! {
            crate::kernel_testmain();
            unreachable!()
        }

        process::current::kernel_thread_spawn(run_tests, &(), Some(ContextPriority::Medium), None)
            .expect("failed to spawn Test Thread");
    }

    thread::current::exit(0)
}

pub fn idle_function() -> ! {
    crate::serial!("entered idle\n");
    loop {
        if SHOULD_WAKEUP.load(Ordering::Acquire) > 0 {
            // A thread yield during this would deadlock if [`schedule_thread_cleanup`] is called
            let mut to_cleanup = TO_CLEANUP.lock();
            // TODO: Maybe there is a faster method to handle this
            to_cleanup.retain(|item| {
                // only remove items that's been around beyond or at `at_context_switch_count`
                match item {
                    CleanupItem::Thread {
                        context_switch_count,
                        at_context_switch_count,
                        thread,
                    } => {
                        if context_switch_count.load(Ordering::Acquire) >= *at_context_switch_count
                        {
                            unsafe { thread.cleanup() };
                            SHOULD_WAKEUP.fetch_sub(1, Ordering::SeqCst);
                            false
                        } else {
                            true
                        }
                    }
                    CleanupItem::Process { proc } => {
                        let mut threads = proc.threads.lock();
                        let can_clean = threads.iter().all(|t| t.is_removed());

                        if can_clean {
                            threads.clear();
                            drop(threads);
                            unsafe {
                                proc.cleanup();
                            }
                        }
                        !can_clean
                    }
                }
            });
        }

        core::hint::spin_loop();
    }
}

/// Schedules a thread's Context for cleanup
/// when the scheduler switches to another thread
/// # Safety
/// If any context switch occurs after this function is called the thread will be dropped
pub unsafe fn schedule_thread_cleanup(
    thread: ArcThread,
    context_switch_count_ref: &'static AtomicUsize,
) {
    let mut to_cleanup = TO_CLEANUP.lock();
    // reserve space for the new item
    to_cleanup.reserve(1);
    to_cleanup.push(CleanupItem::Thread {
        thread,
        context_switch_count: context_switch_count_ref,
        at_context_switch_count: context_switch_count_ref.load(Ordering::Acquire) + 2,
    });
    SHOULD_WAKEUP.fetch_add(1, Ordering::SeqCst);
}

/// Schedules a Process for cleanup
/// when all it's threads are cleaned up
pub fn schedule_proc_cleanup(proc: Arc<Process>) {
    let mut to_cleanup = TO_CLEANUP.lock();
    // reserve space for the new item
    to_cleanup.reserve(1);
    to_cleanup.push(CleanupItem::Process { proc });
    SHOULD_WAKEUP.fetch_add(1, Ordering::SeqCst);
}
