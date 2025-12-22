//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use crate::drivers::driver_poll::{self, PolledDriver};
use crate::memory::paging::PAGE_SIZE;
use crate::scheduler::Scheduler;
use crate::serial;
use crate::thread::{self, ContextPriority, Tid};
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

pub fn main() -> ! {
    *logging::SERIAL_LOG.write() = Some(PageString::with_capacity(&"Journal", PAGE_SIZE * 4));
    crate::info!("eve has been awaken ...");

    crate::drivers::pci::init();

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

    // NOTE: May deadlock because the journal could request memory while lock is held (this is why we allocate 4 pages).
    crate::memory::vmm::with_root(|vmm| vmm.debug_regions());

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
    let scheduler = Scheduler::get().expect("IDLE Started without scheduler");
    scheduler.idle()
}
