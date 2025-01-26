//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use alloc::vec::Vec;
use spin::Mutex;

use crate::{debug, drivers::vfs, memory::paging::PhysPageTable, serial};

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

    pub fn one_shot(&mut self) {
        self.clean_up_list.pop();
    }

    pub fn relaxed(&self) -> bool {
        self.clean_up_list.is_empty()
    }
}

pub static EVE: Mutex<Eve> = Mutex::new(Eve::new());

/// the main loop of Eve
/// it will run until doomsday
pub fn main() -> ! {
    debug!(Eve, "Eve has been awaken ...");
    let stdin = vfs::expose::File::open("dev:/tty").unwrap();
    let stdout = vfs::expose::File::open("dev:/tty").unwrap();
    serial!(
        "Hello, world!, running tests... stdin: {:?}, stdout: {:?}\n",
        stdin,
        stdout
    );

    #[cfg(feature = "test")]
    {
        use crate::threading::expose::{function_spawn, SpawnFlags};
        function_spawn(
            "TestRunner",
            crate::test::main,
            &[],
            SpawnFlags::CLONE_RESOURCES,
        )
        .unwrap();
    }

    loop {
        // TODO: figure out a better method to save cpu time
        // this is a hack to prevent deadlocks
        unsafe { core::arch::asm!("cli") }
        let mut eve = EVE.lock();
        if !eve.relaxed() {
            eve.one_shot();
        }
        drop(eve);
        unsafe { core::arch::asm!("sti") }
    }
}

/// adds a page table to the list of page tables that need to be cleaned up
pub fn add_cleanup(page_table: PhysPageTable) {
    EVE.lock().add_cleanup(page_table);
}
