//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use core::cell::SyncUnsafeCell;
use core::sync::atomic::AtomicUsize;

use crate::arch::{disable_interrupts, enable_interrupts};
use crate::drivers::driver_poll::{self, PolledDriver};
use crate::threading::cpu_context::{Cid, ContextPriority};
use crate::threading::expose::{kernel_thread_spawn, thread_exit};
use crate::utils::alloc::PageString;
use crate::utils::locks::Mutex;
use crate::{debug, logging};
use crate::{drivers::vfs, memory::paging::PhysPageTable, serial};
use alloc::vec::Vec;
use lazy_static::lazy_static;
use safa_utils::{
    abi::raw::processes::{AbiStructures, TaskStdio},
    make_path,
};
use spin::Lazy;

pub struct Eve {
    clean_up_list: Vec<PhysPageTable>,
}

impl Eve {
    const fn new() -> Self {
        Self {
            clean_up_list: Vec::new(),
        }
    }

    pub fn add_cleanup(&mut self, page_table: PhysPageTable) {
        self.clean_up_list.push(page_table);
    }
}

static EVE: Mutex<Eve> = Mutex::new(Eve::new());
static AWAITING_CLEANUP: AtomicUsize = AtomicUsize::new(0);

fn one_shot() {
    unsafe {
        disable_interrupts();
    }
    let mut lock_guard = EVE.lock();
    let item = lock_guard.clean_up_list.pop();
    drop(lock_guard);
    drop(item);
    unsafe {
        enable_interrupts();
    }
}

pub(super) static KERNEL_STDIO: Lazy<TaskStdio> = Lazy::new(|| {
    let stdin = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    let stdout = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    let stderr = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    TaskStdio::new(Some(stdout.fd()), Some(stdin.fd()), Some(stderr.fd()))
});

#[allow(unused)]
static KERNEL_ABI_STRUCTURES: Lazy<AbiStructures> = Lazy::new(|| AbiStructures {
    stdio: *KERNEL_STDIO,
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
            *KERNEL_ABI_STRUCTURES,
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

    kernel_thread_spawn(cleanup_function, &(), Some(ContextPriority::High), None)
        .expect("failed to spawn cleanup thread");
    thread_exit(0)
}

fn cleanup_function(_: u32, (): &()) -> ! {
    loop {
        if AWAITING_CLEANUP.load(core::sync::atomic::Ordering::Acquire) > 0 {
            one_shot();
            AWAITING_CLEANUP.fetch_sub(1, core::sync::atomic::Ordering::Release);
        }
        core::hint::spin_loop();
    }
}

pub fn idle_function() -> ! {
    crate::serial!("entered idle\n");
    loop {
        core::hint::spin_loop();
    }
}

/// adds a page table to the list of page tables that need to be cleaned up
pub fn add_cleanup(page_table: PhysPageTable) {
    EVE.lock().add_cleanup(page_table);
    AWAITING_CLEANUP.fetch_add(1, core::sync::atomic::Ordering::Release);
}
