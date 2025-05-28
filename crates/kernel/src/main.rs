#![no_std]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test::test_runner)]
#![reexport_test_harness_main = "kernel_testmain"]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(allocator_api)]
#![feature(pattern)]
#![feature(const_type_name)]
#![feature(box_vec_non_null)]
#![feature(vec_into_raw_parts)]
#![feature(iter_collect_into)]
#![feature(naked_functions)]
#![feature(sync_unsafe_cell)]
#![feature(never_type)]
#![feature(likely_unlikely)]
#![feature(slice_as_array)]
#![feature(iter_array_chunks)]

#[cfg(test)]
mod test;

mod arch;
mod devices;
mod drivers;
mod eve;
mod globals;
mod limine;
/// Contains macros and stuff related to debugging
/// such as info!, debug! and StackTrace
mod logging;
mod memory;
mod syscalls;
mod terminal;
mod threading;
mod utils;

extern crate alloc;
use arch::serial;

use drivers::keyboard::keys::Key;
use drivers::keyboard::HandleKey;
use globals::*;

pub use memory::PhysAddr;
pub use memory::VirtAddr;
use terminal::FRAMEBUFFER_TERMINAL;
use threading::Scheduler;

#[macro_export]
macro_rules! print {
   ($($arg:tt)*) => ($crate::terminal::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => (print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial {
    ($($arg:tt)*) => {
        $crate::arch::serial::_serial(format_args!($($arg)*))
    };
}

/// Returns the number of milliseconds since the CPU was started
#[macro_export]
macro_rules! time {
    () => {
        $crate::arch::utils::time()
    };
}

#[unsafe(no_mangle)]
pub fn khalt() -> ! {
    loop {
        unsafe { arch::hlt() }
    }
}

#[allow(unused_imports)]
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        arch::disable_interrupts();
    }
    let stack = unsafe { logging::StackTrace::current() };
    unsafe {
        arch::serial::SERIAL.force_unlock();
        if !logging::QUITE_PANIC {
            FRAMEBUFFER_TERMINAL.force_unlock_write();
            FRAMEBUFFER_TERMINAL.write().clear();
        }
    }

    panic_println!(
        "\x1B[31mkernel panic:\n{}, at {}\x1B[0m",
        info.message(),
        info.location().unwrap()
    );
    panic_println!("{}", stack);

    #[cfg(test)]
    arch::power::shutdown();
    #[cfg(not(test))]
    khalt();
}

#[no_mangle]
extern "C" fn kstart() -> ! {
    arch::init_phase1();
    memory::sorcery::init_page_table();
    info!("terminal initialized");
    logging::BOOTING.store(true, core::sync::atomic::Ordering::Relaxed);
    // initing the arch
    arch::init_phase2();

    unsafe {
        debug!(Scheduler, "Eve starting...");
        logging::BOOTING.store(false, core::sync::atomic::Ordering::Relaxed);
        Scheduler::init(eve::main, "Eve");
    }

    #[allow(unreachable_code)]
    {
        panic!("failed context switching to Eve! ...")
    }
}

// whenever a key is pressed this function should be called
// this executes a few other kernel-functions
pub fn __navi_key_pressed(key: Key) {
    if let Some(mut writer) = FRAMEBUFFER_TERMINAL.try_write() {
        writer.handle_key(key);
    };
}
