use core::mem::MaybeUninit;

use lazy_static::lazy_static;
use spin::Mutex;

use crate::{
    limine,
    memory::buddy_allocator::BuddyAllocator,
    utils::{self, elf::Elf, Locked},
};

#[global_allocator]
static GLOBAL_ALLOCATOR: Locked<MaybeUninit<BuddyAllocator>> =
    unsafe { Locked::new(BuddyAllocator::new()) };

pub fn global_allocator() -> &'static Mutex<MaybeUninit<BuddyAllocator<'static>>> {
    &GLOBAL_ALLOCATOR.inner
}
/// static mut because we need really fast access of HDDM
pub static mut HDDM: usize = 0;
#[inline(always)]
pub fn hddm() -> usize {
    unsafe { HDDM }
}

lazy_static! {
    pub static ref KERNEL_ELF: Elf<'static> = {
        let kernel_img = limine::kernel_image_info();
        let kernel_img_bytes = unsafe { core::slice::from_raw_parts(kernel_img.0, kernel_img.1) };
        let elf = utils::elf::Elf::new(kernel_img_bytes).unwrap();
        elf
    };
    pub static ref RSDP_ADDR: usize = limine::rsdp_addr();
}
