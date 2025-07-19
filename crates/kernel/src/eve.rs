//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use core::cell::SyncUnsafeCell;

use crate::drivers::driver_poll::{self, PolledDriver};
use crate::threading::cpu_context::{Cid, ContextPriority};
use crate::threading::expose::{kernel_thread_spawn, thread_exit};
use crate::utils::alloc::PageString;
use crate::{debug, logging};
use crate::{drivers::vfs, serial};
use alloc::vec::Vec;
use lazy_static::lazy_static;
use safa_utils::{abi::raw::processes::ProcessStdio, make_path};
use spin::Lazy;

pub(super) static KERNEL_STDIO: Lazy<ProcessStdio> = Lazy::new(|| {
    let stdin = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    let stdout = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    let stderr = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    ProcessStdio::new(Some(stdout.fd()), Some(stdin.fd()), Some(stderr.fd()))
});

lazy_static! {
    static ref POLLING: SyncUnsafeCell<Vec<&'static dyn PolledDriver>> =
        SyncUnsafeCell::new(driver_poll::take_poll());
}

fn poll_driver_thread(cid: Cid, driver: &&dyn PolledDriver) -> ! {
    debug!(
        "polling driver in thread: {}, thread CID: {cid}",
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
        kernel_thread_spawn(
            poll_driver_thread,
            poll_driver,
            Some(ContextPriority::High),
            Some(0),
        )
        .expect("failed to spawn a thread function for a polled driver");
    }

    #[cfg(not(test))]
    {
        use crate::threading::expose::SpawnFlags;
        use crate::{info, sleep, threading::expose::pspawn};
        use safa_utils::types::Name;

        info!(
            "kernel finished boot, waiting a delay of 2.5 second(s), FIXME: fix needing hardcoded delay to let the XHCI finish before the Shell"
        );
        sleep!(2500 ms);

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
        use crate::threading::cpu_context::ContextPriority;

        fn run_tests(_cid: Cid, _arg: &()) -> ! {
            crate::kernel_testmain();
            unreachable!()
        }

        kernel_thread_spawn(run_tests, &(), Some(ContextPriority::Medium), None)
            .expect("failed to spawn Test Thread");
    }

    thread_exit(0)
}

pub fn idle_function() -> ! {
    crate::serial!("entered idle\n");
    loop {
        core::hint::spin_loop();
    }
}
