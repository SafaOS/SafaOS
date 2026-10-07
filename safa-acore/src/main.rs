#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test::test_runner)]
#![reexport_test_harness_main = "kernel_testmain"]
#![feature(const_ops)]
#![feature(const_trait_impl)]
#![feature(sync_unsafe_cell)]
#![feature(allocator_api)]
#![cfg_attr(test, feature(const_type_name))]

extern crate alloc;
mod arch;
mod bootloader;
mod eve;
mod logging;
mod mem;
mod misc;
mod oninit;
mod paging;
mod percpu;
mod scheduler;
mod sync;
mod task;
#[cfg(test)]
mod test;
mod time;

use core::panic::PanicInfo;

use crate::{bootloader::HHDM, eve::eve_main};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { logging::panic_mode() };
    logging::fatal!("panic", "{}", info);
    loop {
        arch::halt()
    }
}

#[unsafe(no_mangle)]
extern "C" fn kstart() -> ! {
    arch::boot::kboot()
}

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    unsafe { oninit::init() };
    arch::boot::init_phase1();

    logging::info!("boot", "Phase 1 completed HHDM={:?}", &*HHDM);
    for mmap in bootloader::memory_map() {
        logging::debug!("boot", "memory map: {:#?}", mmap);
    }

    logging::init();

    #[cfg(test)]
    crate::kernel_testmain();

    task::spawn(eve_main());
    scheduler::schedule_loop();
    panic!("How did we get here?!")
}
