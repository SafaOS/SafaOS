//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use crate::drivers::driver_poll;
use crate::utils::locks::Mutex;
use crate::{drivers::vfs, memory::paging::PhysPageTable, serial, threading::expose::thread_yield};
use alloc::vec::Vec;
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

/// the main loop of Eve
/// it will run until doomsday
pub fn main() -> ! {
    crate::info!("eve has been awaken ...");
    // TODO: make a macro or a const function to do this automatically
    serial!("Hello, world!, running tests...\n",);

    #[cfg(test)]
    {
        use crate::threading::expose::{function_spawn, SpawnFlags};
        use crate::utils::types::Name;

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

    #[cfg(not(test))]
    {
        use crate::threading::expose::{pspawn, SpawnFlags};
        use crate::utils::types::Name;

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
        for driver in &*driver_poll::read_poll() {
            // TODO: spawn in a thread and poll every PolledDriver::poll_every
            driver.poll();
        }
        core::hint::spin_loop();
    }
}

/// adds a page table to the list of page tables that need to be cleaned up
pub fn add_cleanup(page_table: PhysPageTable) {
    EVE.lock().add_cleanup(page_table);
}
