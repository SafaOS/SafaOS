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

// extern crate alloc;
mod arch;
mod bootloader;
mod logging;
mod memory;
mod misc;
mod oninit;
mod paging;
mod percpu;
mod sync;
#[cfg(test)]
mod test;
mod time;

use core::panic::PanicInfo;

use crate::bootloader::HHDM;

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
    unsafe { memory::init_pmm() };
    unsafe { memory::init_alloc() };
    arch::boot::init_phase1();

    logging::info!("boot", "Phase 1 completed HHDM={:?}", &*HHDM);
    for mmap in bootloader::memory_map() {
        logging::debug!("boot", "memory map: {:#?}", mmap);
    }

    logging::init();

    #[cfg(test)]
    crate::kernel_testmain();
    panic!("How did we get here?!")
}
