extern "C" fn bsp_init() {
    let bsp = crate::percpu::init_bsp_first();
    unsafe { super::registers::set_cpu_local(bsp) };
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn kboot() -> ! {
    core::arch::naked_asm!(
        "
    mov x0, sp
    # Enables SP_ELx
    mrs x1, spsel
    orr x1, x1, #1
    msr spsel, x1
    # Restores the stack back after enabling
    mov sp, x0

    bl {}
    b kmain
    udf #0
    ", sym bsp_init
    )
}
