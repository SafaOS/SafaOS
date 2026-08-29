use crate::{logging, percpu};

extern "C" fn bsp_init() {
    let bsp = percpu::init_bsp_first();
    super::gdt::init_gdt(bsp);
    super::serial::init_serial_inner();

    logging::sprintln!("GDT init... Ok\n");
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn kboot() -> ! {
    core::arch::naked_asm!(
        "
    call {}
    jmp kmain
    ud2
    ", sym bsp_init
    )
}

pub fn init_phase1() {}
