//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use core::cell::SyncUnsafeCell;

use crate::drivers::driver_poll::{self, PolledDriver};
use crate::threading;
use crate::threading::expose::{function_spawn, SpawnFlags};
use crate::utils::locks::Mutex;
use crate::{drivers::vfs, memory::paging::PhysPageTable, serial, threading::expose::thread_yield};
use alloc::vec::Vec;
use lazy_static::lazy_static;
use safa_utils::types::Name;
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

pub static EVE: Mutex<Eve> = Mutex::new(Eve::new());

fn one_shot() -> Option<PhysPageTable> {
    loop {
        match EVE.try_lock() {
            Some(mut eve) => return eve.clean_up_list.pop(),
            None => thread_yield(),
        }
    }
}

pub static KERNEL_STDIO: Lazy<TaskStdio> = Lazy::new(|| {
    let stdin = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    let stdout = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    let stderr = vfs::expose::FileRef::open(make_path!("dev", "tty")).unwrap();
    TaskStdio::new(Some(stdout.fd()), Some(stdin.fd()), Some(stderr.fd()))
});

pub static KERNEL_ABI_STRUCTURES: Lazy<AbiStructures> = Lazy::new(|| AbiStructures {
    stdio: *KERNEL_STDIO,
});

lazy_static! {
    static ref POLLING: SyncUnsafeCell<Vec<&'static dyn PolledDriver>> =
        SyncUnsafeCell::new(driver_poll::take_poll());
}

fn poll_driver_thread() -> ! {
    let current = threading::current();
    let thread_name = current.name();
    let mut poll_driver = None;
    for polled_driver in unsafe { &*POLLING.get() } {
        if polled_driver.thread_name() == thread_name {
            poll_driver = Some(polled_driver);
        }
    }

    let poll_driver = poll_driver.unwrap_or_else(|| {
        panic!(
            "failed to find a polled driver for the thread: {}",
            thread_name
        )
    });
    drop(current);
    poll_driver.poll_function()
}

/// the main loop of Eve
/// it will run until doomsday
pub fn main() -> ! {
    crate::info!("eve has been awaken ...");
    // TODO: make a macro or a const function to do this automatically
    serial!("Hello, world!, running tests...\n",);

    #[cfg(test)]
    {
        fn run_tests() -> ! {
            crate::kernel_testmain();
            unreachable!()
        }

        function_spawn(
            Name::try_from("TestRunner").unwrap(),
            run_tests,
            &[],
            &[],
            SpawnFlags::CLONE_RESOURCES,
            *KERNEL_ABI_STRUCTURES,
        )
        .unwrap();
    }

    // FIXME: use threads
    for poll_driver in unsafe { &*POLLING.get() } {
        function_spawn(
            Name::try_from(poll_driver.thread_name()).unwrap(),
            poll_driver_thread,
            &[],
            &[],
            SpawnFlags::CLONE_RESOURCES,
            *KERNEL_ABI_STRUCTURES,
        )
        .expect("failed to spawn a fucntion for a polled driver");
    }

    #[cfg(not(test))]
    {
        use crate::threading::expose::pspawn;

        // start the shell
        pspawn(
            Name::try_from("Shell").unwrap(),
            // Maybe we can make a const function or a macro for this
            make_path!("sys", "bin/safa"),
            &["sys:/bin/safa", "-i"],
            &[b"PATH=sys:/bin", b"SHELL=sys:/bin/safa"],
            SpawnFlags::empty(),
            *KERNEL_ABI_STRUCTURES,
        )
        .unwrap();
    }

    loop {
        one_shot();
        core::hint::spin_loop();
    }
}

/// adds a page table to the list of page tables that need to be cleaned up
pub fn add_cleanup(page_table: PhysPageTable) {
    EVE.lock().add_cleanup(page_table);
}
