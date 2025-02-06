#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(allocator_api)]
#![feature(try_trait_v2)]
#![feature(pattern)]
#![feature(box_vec_non_null)]
#![feature(vec_into_raw_parts)]
#![feature(iter_collect_into)]

#[cfg(feature = "test")]
mod test;

mod arch;
mod devices;
mod drivers;
mod eve;
mod globals;
mod limine;
mod memory;
mod syscalls;
mod terminal;
mod threading;
mod utils;

extern crate alloc;
use arch::x86_64::serial;

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
        $crate::arch::x86_64::serial::_serial(format_args!($($arg)*))
    };
}

/// Returns the number of milliseconds since the CPU was started
#[macro_export]
macro_rules! time {
    () => {
        $crate::arch::x86_64::utils::time()
    };
}

use core::arch::asm;
#[no_mangle]
#[inline]
pub fn khalt() -> ! {
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            asm!("hlt")
        }
    }
}

#[allow(unused_imports)]
use core::panic::PanicInfo;
pub const QUITE_PANIC: bool = true;

/// prints to both the serial and the terminal doesn't print to the terminal if it panicked or if
/// it is not ready...
#[macro_export]
macro_rules! cross_println {
    ($($arg:tt)*) => {
        $crate::serial!($($arg)*);
        $crate::serial!("\n");

        if !$crate::QUITE_PANIC {
            $crate::println!($($arg)*);
        }
    };
}

/// runtime debug info that is only avalible though test feature
/// takes a $mod and an Arguments, mod must be a type
#[macro_export]
macro_rules! debug {
    ($mod: path, $($arg:tt)*) => {
        // makes sure $mod is a vaild type
        let _ = core::marker::PhantomData::<$mod>;
        $crate::serial!("\x1B[38;2;0;155;200m[DEBUG]\x1B[38;2;255;155;0m {}: \x1B[0m{}\n", stringify!($mod), format_args!($($arg)*));
    };
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { asm!("cli") }
    unsafe {
        arch::x86_64::serial::SERIAL.force_unlock();
        if !QUITE_PANIC {
            FRAMEBUFFER_TERMINAL.force_write_unlock();
            FRAMEBUFFER_TERMINAL.write().clear();
        }
    }

    cross_println!(
        "\x1B[38;2;255;0;0mkernel panic:\n{}, at {}\x1B[0m",
        info.message(),
        info.location().unwrap()
    );
    print_stack_trace();

    // crate::serial!("tty stdout dump:\n{}\n", crate::terminal().stdout_buffer);
    // crate::serial!("tty stdin dump:\n{}\n", crate::terminal().stdin_buffer);
    khalt()
}

#[allow(unused)]
fn print_stack_trace() {
    let mut fp: *const usize;

    unsafe {
        core::arch::asm!("mov {}, rbp", out(reg) fp);

        cross_println!("\x1B[38;2;0;0;200mStack trace:");
        while !fp.is_null() && fp.is_aligned() {
            let return_address_ptr = fp.offset(1);
            let return_address = *return_address_ptr;

            let name = {
                let sym = KERNEL_ELF.sym_from_value_range(return_address);
                sym.map(|sym| KERNEL_ELF.string_table_index(sym.name_index).unwrap())
            };
            let name = name.as_deref().unwrap_or("???");

            cross_println!("  {:#x} <{}>", return_address, name);
            fp = *fp as *const usize;
        }
        cross_println!("\x1B[0m");
    }
}

#[no_mangle]
extern "C" fn kstart() -> ! {
    arch::init_phase1();
    memory::sorcery::init_page_table();
    println!("Terminal initialized successfuly");

    // initing the arch
    arch::init_phase2();

    unsafe {
        debug!(Scheduler, "Eve starting...");
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
