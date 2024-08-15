#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(iter_advance_by)]
#![feature(const_mut_refs)]
#![feature(custom_test_frameworks)]
#![feature(proc_macro_hygiene)]
#![feature(asm_const)]
#[cfg(feature = "test")]
mod test;

mod arch;
mod drivers;
mod globals;
mod memory;
mod terminal;
mod threading;
mod utils;

extern crate alloc;
use arch::threading::restore_cpu_status;
use arch::x86_64::serial;
use bootloader_api::info::MemoryRegions;

use drivers::keyboard::Key;
use globals::*;

use memory::frame_allocator::RegionAllocator;
use memory::paging::Mapper;
pub use memory::PhysAddr;
pub use memory::VirtAddr;
use terminal::framebuffer::Terminal;
use threading::Scheduler;
#[macro_export]
macro_rules! print {
   ($($arg:tt)*) => ($crate::terminal::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => (print!("\n"));
    ($($arg:tt)*) => (crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial {
    ($($arg:tt)*) => {
        crate::arch::x86_64::serial::_serial(format_args!($($arg)*))
    };
}

use core::arch::asm;
#[allow(unused_imports)]
use core::panic::PanicInfo;

#[allow(dead_code)]
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { asm!("cli") }
    serial!("kernel panic: ");
    serial!("{}, at {}", info.message(), info.location().unwrap());

    if terminal_inited() {
        println!("\\[fg: (255, 0, 0) |\nkernel panic: |]");
        println!("{}, at {}", info.message(), info.location().unwrap());

        println!("\\[fg: (255, 0, 0) |cannot continue execution kernel will now hang|]");
    }
    loop {}
}

pub extern "C" fn kinit(bootinfo: &'static mut bootloader_api::BootInfo) {
    // initing globals
    let phy_offset = &mut bootinfo.physical_memory_offset;
    let phy_offset = phy_offset.as_mut().unwrap();

    let regions: &'static mut MemoryRegions = &mut bootinfo.memory_regions;

    unsafe {
        RSDP_ADDR = bootinfo.rsdp_addr.into();
        FRAME_ALLOCATOR = Some(RegionAllocator::new(&mut *regions));
        PHY_OFFSET = *phy_offset as usize;
        let mapper = Mapper::new(*phy_offset as usize);
        PAGING_MAPPER = Some(mapper);
    };

    // initing the arch
    arch::init();
    unsafe {
        serial!(
            "image: 0x{:x}\nlen: 0x{:x}\nphy_offset: 0x{:x}\n",
            bootinfo.kernel_image_offset,
            bootinfo.kernel_len,
            phy_offset
        );

        memory::init_memory((bootinfo.kernel_image_offset + bootinfo.kernel_len + 1) as usize)
            .unwrap();

        let terminal: Terminal<'static> = Terminal::init(bootinfo.framebuffer.as_mut().unwrap());
        TERMINAL = Some(terminal);
    }

    serial!("kernel init phase 1 done\n");

    unsafe {
        asm!("cli");
        let mut scheduler = Scheduler::init(kidle as usize, "kernel");

        scheduler.create_process(terminal::shell as usize, "shell");
        SCHEDULER = Some(scheduler);

        restore_cpu_status(&(*SCHEDULER.as_ref().unwrap().current_process).context)
    }
}

#[no_mangle]
fn kmain(boot_info: &'static mut bootloader_api::BootInfo) -> ! {
    let rsp: u64;
    unsafe {
        asm!("mov {}, rsp", out(reg) rsp);
    }
    serial!("rsp: 0x{:x}\n", rsp);

    kinit(boot_info);
    serial!("failed context switching to kidle! ...\n");
    loop {}
}

fn kidle() -> ! {
    serial!("Hello, world!, running tests...\n");

    #[cfg(feature = "test")]
    test::testing_module::test_main();

    println!(
        "\\[fg: (0, 255, 0) ||Boot success! press ctrl + shift + C to clear screen (and enter input mode)\n||]"
    );

    serial!("finished initing...\n");
    serial!("idle!\n");

    loop {
        unsafe {
            asm!("hlt");
        }
    }
}

// /// does some pooling and stuff stops interrupts to do it's work first!
// fn kwork() {
//     serial!("work!\n");
//     loop {
//         // unsafe { asm!("cli") }
//         // #[cfg(target_arch = "x86_64")]
//         // arch::x86_64::interrupts::handlers::handle_ps2_keyboard();
//         // unsafe { asm!("sti") }
//     }
// }

// whenever a key is pressed this function should be called
// this executes a few other kernel-functions
pub fn __navi_key_pressed(key: Key) {
    if globals::terminal_inited() {
        terminal().on_key_pressed(key)
    }
}

static CONFIG: bootloader_api::BootloaderConfig = {
    use bootloader_api::{
        config::{Mapping, Mappings},
        BootloaderConfig,
    };

    let mut config = BootloaderConfig::new_default();
    let mut mappings = Mappings::new_default();
    mappings.physical_memory = Some(Mapping::Dynamic);
    mappings.dynamic_range_start = Some(0xffff_8000_0000_0000);
    config.mappings = mappings;
    config
};
bootloader_api::entry_point!(kmain, config = { &CONFIG });
