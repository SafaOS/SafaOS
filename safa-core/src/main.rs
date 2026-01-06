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
#![feature(sync_unsafe_cell)]
#![feature(never_type)]
#![feature(likely_unlikely)]
#![feature(slice_as_array)]
#![feature(iter_array_chunks)]
#![feature(const_trait_impl)]
#![feature(const_ops)]
#![feature(unsafe_cell_access)]
#![feature(macro_metavar_expr_concat)]
#![feature(set_ptr_value)]
#![feature(debug_closure_helpers)]

#[cfg(test)]
mod test;

mod arch;
mod devices;
mod drivers;
mod eve;
mod fs;
mod globals;
mod limine;
/// Contains macros and stuff related to debugging
/// such as info!, debug! and StackTrace
mod logging;
mod memory;
mod net;
mod percpu;
mod process;
mod scheduler;
mod shared_mem;
mod smp;
mod sockets;
mod syscalls;
mod terminal;
mod thread;
mod timer;
mod utils;
mod vtty;

extern crate alloc;
use arch::serial::{self, SERIAL};

use globals::*;

pub use memory::PhysAddr;
pub use memory::VirtAddr;
use terminal::FRAMEBUFFER_TERMINAL;

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

#[macro_export]
/// Sleeps n ms
///
/// vatiants:
///
/// sleep!(N ms)
/// sleep!(N) (ms)
macro_rules! sleep {
    ($ms: expr_2021) => {{
        use $crate::timer::SystemInstant;
        let instant = SystemInstant::now();

        while instant.elapsed().as_millis() < $ms as u128 {
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
///
/// sleep_until(timeout ms, let var = expr; until condition)
/// returns Some(var) if not timeouted, condition may use that var also.
macro_rules! sleep_until {
    ($cond: tt) => {{
        while !$cond {
            core::hint::spin_loop()
        }

        true
    }};

    ($timeout_ms: literal ms, $cond: expr_2021) => {{
        use $crate::timer::SystemInstant;
        let instant = SystemInstant::now();

        let mut success = true;
        while !$cond {
            if instant.elapsed().as_millis() >= $timeout_ms as u128 {
                success = $cond;
                break;
            }

            core::hint::spin_loop();
        }

        success
    }};

    ($timeout_ms: literal ms, let $name: ident = $expr: expr; until $cond: expr) => {{
        use $crate::timer::SystemInstant;
        let instant = SystemInstant::now();

        let mut $name = $expr;
        while !$cond {
            if instant.elapsed().as_millis() >= $timeout_ms as u128 {
                break;
            }

            $name = $expr;
            core::hint::spin_loop();
        }

        $cond.then_some($name)
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
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use crate::arch::registers::ArchCpuID;
use crate::arch::without_interrupts;
use crate::smp::READY_CPUS;
use crate::utils::locks::SpinLock;

static PANCIKED: AtomicUsize = AtomicUsize::new(0);
const MAX_PANICK_COUNT: usize = 3;
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let stack = unsafe { logging::StackTrace::current() };
    without_interrupts(|| {
        unsafe {
            arch::halt_all();
        }

        // Wait for halt to complete
        if READY_CPUS.load(Ordering::SeqCst) > 1 {
            crate::sleep!(10 ms);
        }
        static _PANICK_LOCK: SpinLock<()> = SpinLock::new(());
        let _guard = _PANICK_LOCK.lock();

        if PANCIKED.fetch_add(1, core::sync::atomic::Ordering::Release) >= MAX_PANICK_COUNT {
            unsafe {
                SERIAL.force_unlock();
            }
            error!(
                "\n\x1B[31mkernel :3 panic within a panic:\n{info}, cpu: {}\n\x1B[0mno stack trace",
                ArchCpuID::get()
            );
            khalt()
        }

        unsafe {
            SERIAL.force_unlock();
            if !logging::QUITE_PANIC {
                FRAMEBUFFER_TERMINAL.force_unlock_write();
                FRAMEBUFFER_TERMINAL.write().clear();
            }
        }

        panic_println!(
            "\x1B[31mkernel :3 panic:\n{}, at {}, cpu: {}\x1B[0m",
            info.message(),
            info.location().unwrap(),
            ArchCpuID::get(),
        );
        panic_println!("{}", stack);

        drop(_guard);
        #[cfg(test)]
        arch::power::shutdown();
        #[cfg(not(test))]
        khalt();
    })
}

/// Basic scheduler, memory, and CPU initialization.
/// The reset of the initialization is done by [`eve::main`].
#[unsafe(no_mangle)]
extern "C" fn kstart() -> ! {
    arch::init_phase1();
    memory::init::init_all();
    println!("Terminal Initialized");
    logging::BOOTING.store(true, core::sync::atomic::Ordering::Relaxed);
    // initing the arch
    arch::init_phase2();

    unsafe {
        logging::BOOTING.store(false, core::sync::atomic::Ordering::Relaxed);
        scheduler::init(eve::main, "Eve");
    }

    #[allow(unreachable_code)]
    {
        panic!("failed context switching to Eve! ...")
    }
}
