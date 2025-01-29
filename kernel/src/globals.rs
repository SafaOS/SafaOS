use lazy_static::lazy_static;

use crate::{
    limine,
    utils::{self, elf::Elf},
};

lazy_static! {
    pub static ref KERNEL_ELF: Elf<'static> = {
        let kernel_img = limine::kernel_image_info();
        let kernel_img_bytes = unsafe { core::slice::from_raw_parts(kernel_img.0, kernel_img.1) };
        let elf = utils::elf::Elf::new(kernel_img_bytes).unwrap();
        elf
    };
    pub static ref RSDP_ADDR: usize = limine::rsdp_addr();
}

pub const KERNEL_CODE_NAME: &str = "Snowball";
pub const KERNEL_CODE_VERSION: &str = "0.1.0";
