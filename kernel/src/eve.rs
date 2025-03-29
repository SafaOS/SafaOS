//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use crate::{
    debug, drivers::vfs, memory::paging::PhysPageTable, serial, threading::expose::thread_yeild,
};
use alloc::vec::Vec;
use safa_utils::make_path;
use spin::Mutex;

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
            None => thread_yeild(),
        }
    }
}

/// the main loop of Eve
/// it will run until doomsday
pub fn main() -> ! {
    debug!(Eve, "Eve has been awaken ...");
    // TODO: make a macro or a const function to do this automatically
    let stdin = vfs::expose::File::open(make_path!("dev", "tty")).unwrap();
    let stdout = vfs::expose::File::open(make_path!("dev", "tty")).unwrap();
    serial!(
        "Hello, world!, running tests... stdin: {:?}, stdout: {:?}\n",
        stdin,
        stdout
    );

    #[cfg(feature = "test")]
    {
        use crate::threading::expose::{function_spawn, SpawnFlags};
        use crate::utils::Name;

        function_spawn(
            Name::try_from("TestRunner").unwrap(),
            crate::test::main,
            &[],
            SpawnFlags::CLONE_RESOURCES,
        )
        .unwrap();
    }

    loop {
        one_shot();
    }
}

/// adds a page table to the list of page tables that need to be cleaned up
pub fn add_cleanup(page_table: PhysPageTable) {
    loop {
        match EVE.try_lock() {
            Some(mut eve) => {
                eve.add_cleanup(page_table);
                return;
            }
            None => {
                thread_yeild();
            }
        }
    }
}
