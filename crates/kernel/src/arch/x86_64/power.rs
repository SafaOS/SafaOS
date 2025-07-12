use core::arch::asm;

use crate::{arch::x86_64::gdt, println, serial};

use super::{acpi, outb};

// const SLP_TYP_S5: u16 = 0x1C00;
// const SLP_EN: u16 = 1 << 13;

pub fn shutdown() -> ! {
    loop {}
    // let fadt = *acpi::FADT_DESC;

    // let pm1a_cnt_blk = fadt.pm1a_cnt_blk as u16;
    // let shutdown_command = SLP_TYP_S5 | SLP_EN;
    // outw(pm1a_cnt_blk, shutdown_command);

    // if fadt.pm1b_cnt_blk != 0 {
    //     let pm1b_cnt_blk = fadt.pm1b_cnt_blk as u16;
    //     outw(pm1b_cnt_blk, shutdown_command);
    // }

    // // if failed to shutdown shutdown qemu!
    // outw(0xB004, 0x2000);
    // outw(0x604, 0x2000);
    // unreachable!()
}

pub fn reboot() -> ! {
    unsafe { asm!("cli") };

    let fadt = *acpi::FADT_DESC;
    match fadt.reset_reg.address_space {
        1 => outb(fadt.reset_reg.address as u16, fadt.reset_value),
        _ => serial!("unknown fadt reset_reg? {:#?}\n", fadt.reset_reg),
    }

    // force-reboot because acpi sucks!
    let x = 0;
    unsafe { asm!("mov al, {}\nmov ss, ax", in(reg_byte) gdt::KERNEL_DATA_SEG) }
    unsafe { asm!("lidt [{}]", in(reg) &x) };
    unsafe { asm!("int3") };

    println!("failed to reboot maybe your device is not supported yet?");

    unreachable!()
}
