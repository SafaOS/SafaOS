use lazy_static::lazy_static;

use crate::{
    limine,
    utils::{self, elf::Elf},
};

lazy_static! {
    static ref KERNEL_ELF_BYTES: &'static [u8] = {
        let kernel_img = limine::kernel_image_info();
        unsafe { core::slice::from_raw_parts(kernel_img.0, kernel_img.1) }
    };
    pub static ref KERNEL_ELF: Elf<'static, &'static [u8]> =
        utils::elf::Elf::new(&*KERNEL_ELF_BYTES).unwrap();
    pub static ref RSDP_ADDR: usize = limine::rsdp_addr();
}

pub const KERNEL_CODE_NAME: &str = "Snowball";
pub const KERNEL_CODE_VERSION: &str = "0.1.0";
