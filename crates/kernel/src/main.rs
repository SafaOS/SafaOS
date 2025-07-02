#![no_std]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test::test_runner)]
#![reexport_test_harness_main = "kernel_testmain"]
#![no_main]
#![feature(cold_path)]
#![feature(abi_x86_interrupt)]
#![feature(allocator_api)]
#![feature(pattern)]
#![feature(const_type_name)]
#![feature(box_vec_non_null)]
#![feature(vec_into_raw_parts)]
#![feature(iter_collect_into)]
#![feature(let_chains)]
#![feature(sync_unsafe_cell)]
#![feature(never_type)]
#![feature(likely_unlikely)]
#![feature(slice_as_array)]
#![feature(iter_array_chunks)]
#![feature(const_trait_impl)]
#![feature(const_ops)]
#![feature(unsafe_cell_access)]
#![feature(macro_metavar_expr_concat)]

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

use drivers::keyboard::HandleKey;
use drivers::keyboard::keys::Key;
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
    (ms) => {
        $crate::arch::utils::time_ms()
    };
    (us) => {
        $crate::arch::utils::time_us()
    };
}

#[macro_export]
/// Sleeps n ms
///
/// vatiants:
///
/// sleep!(N ms)
/// sleep!(N) (ms)
macro_rules! sleep {
    ($ms: expr_2021) => {{
        let start_time = $crate::time!(ms);
        let timeout_time = start_time + $ms as u64;

        while $crate::time!(ms) < timeout_time {
            core::hint::spin_loop()
        }
    }};
    ($ms: literal ms) => {{ $crate::sleep!($ms) }};
}

#[macro_export]
/// Sleeps until condition is true
/// variants:
///
/// sleep_until!(condition)
///
/// sleep_until!(timeout ms, condition)
///
/// both returns true if condition happened to be successful, on timeout returns false
macro_rules! sleep_until {
    ($cond: tt) => {{
        while !$cond {
            core::hint::spin_loop()
        }

        true
    }};

    ($timeout_ms: literal ms, $cond: expr_2021) => {{
        let start_time = $crate::time!(ms);
        let timeout_time = start_time + $timeout_ms;
        let mut success = true;

        while !$cond {
            if $crate::time!(ms) >= timeout_time {
                success = $cond;
                break;
            }

            core::hint::spin_loop();
        }

        success
    }};
}

#[unsafe(no_mangle)]
pub fn khalt() -> ! {
    loop {
        unsafe { arch::hlt() }
    }
}

#[allow(unused_imports)]
use core::panic::PanicInfo;
use core::sync::atomic::AtomicBool;

static PANCIKED: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        arch::disable_interrupts();
    }

    if PANCIKED.load(core::sync::atomic::Ordering::Relaxed) {
        unsafe { arch::serial::SERIAL.force_unlock() };
        error!("\x1B[31mkernel panic within a panic:\n{info}\n\x1B[0mno stack trace");
        khalt()
    }
    PANCIKED.store(true, core::sync::atomic::Ordering::Relaxed);

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

#[unsafe(no_mangle)]
extern "C" fn kstart() -> ! {
    arch::init_phase1();
    memory::sorcery::init_page_table();
    info!("terminal initialized");
    logging::BOOTING.store(true, core::sync::atomic::Ordering::Relaxed);
    // initing the arch
    arch::init_phase2();
    drivers::pci::init();

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
