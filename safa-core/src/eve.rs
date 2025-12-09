//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use crate::arch::smp::CPULocal;
use crate::drivers::driver_poll::{self, PolledDriver};
use crate::process::current::kernel_thread_spawn;
use crate::scheduler::{Scheduler, ThreadScheduleReason, cpu_count};
use crate::serial;
use crate::thread::{self, ArcThread, ContextPriority, Tid};
use crate::utils::alloc::PageString;
use crate::utils::path::make_path;
use crate::{debug, fs, logging, process};
use alloc::vec::Vec;
use core::cell::SyncUnsafeCell;
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

fn poll_driver_thread(tid: Tid, driver: &&dyn PolledDriver) -> ! {
    debug!(
        "polling driver in thread: {}, thread TID: {tid}",
        driver.thread_name()
    );
    driver.poll_function()
}

fn thread_reaper_thread(tid: Tid, scheduler: &Scheduler) -> ! {
    debug!("Thread cleaner spawned tid: {tid}");
    loop {
        // The idea is once we reach this point, no other thread is currently scheduling for execution with in this scheduler.
        // So we can safely clean up all threads.
        //
        // TODO: I found this to be faster than cleaning up in a different thread, even though cleaning up in a different thread could allow for immediate cleanup on SMP.
        scheduler.cleanup_all_and_wait();
    }
}

pub fn main() -> ! {
    *logging::SERIAL_LOG.write() = Some(PageString::new());
    crate::info!("eve has been awaken ...");

    crate::drivers::pci::init();

    for cpu in 0..cpu_count() {
        let cpu_local = &CPULocal::get_all()[cpu];
        let scheduler = cpu_local
            .scheduler()
            .expect("Schedulers should be initialized before calling eve");

        kernel_thread_spawn(
            thread_reaper_thread,
            scheduler,
            Some(ContextPriority::Immediate),
            Some(cpu),
        )
        .expect("Failed to spawn a cleanup thread");
    }

    // TODO: make a macro or a const function to do this automatically

    for poll_driver in unsafe { &*POLLING.get() } {
        process::current::kernel_thread_spawn(
            poll_driver_thread,
            poll_driver,
            Some(ContextPriority::High),
            Some(0),
        )
        .expect("failed to spawn a thread function for a polled driver");
    }

    serial!("Hello, world!, running tests...\n",);

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
    crate::khalt()
}

/// Schedules a thread's Context for cleanup
/// when the scheduler switches to another thread
/// # Safety
/// If any context switch occurs after this function is called the thread will be dropped
pub unsafe fn schedule_thread_cleanup(thread: ArcThread) {
    let scheduler = thread.scheduler();
    unsafe { scheduler.schedule_thread(thread, ThreadScheduleReason::Cleanup) };
}
