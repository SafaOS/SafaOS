use core::{
    arch::{asm, global_asm},
    fmt::Display,
};

use crate::VirtAddr;

use super::registers::{Esr, GeneralPurpose};

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct InterruptFrame {
    general_registers: [GeneralPurpose; 19],
    fp: GeneralPurpose,
    lr: GeneralPurpose,
    xzr: GeneralPurpose,
    esr: Esr,
    far: GeneralPurpose,
}

impl Display for InterruptFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Saved general purpose registers:")?;
        let cols = 3;
        let rows = self.general_registers.len() / cols;

        let write = |index: usize, f: &mut core::fmt::Formatter<'_>| {
            write!(f, "x{index}: {:#016x}    ", *self.general_registers[index])
        };

        let mut left = self.general_registers.len();

        for row in 0..rows {
            for col in 0..cols {
                let index = (row * cols) + col;
                write(index, f)?;
                left -= 1;
            }
            writeln!(f)?;
        }

        for i in 0..left {
            let b = (rows * cols) + i;
            write(b, f)?;
        }

        writeln!(f)?;

        writeln!(f, "Special Registers:")?;

        writeln!(f, "{}", self.esr)?;
        write!(f, "FAR: {:?}", self.far)?;
        Ok(())
    }
}

global_asm!(
    "
.text
.global exc_vector_table
exception_entry:
    sub sp, sp, #192
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x29, [sp, #144]
    stp x30, xzr, [sp, #160]

    mrs x0, ESR_EL1
    mrs x1, FAR_EL1
    stp x0, x1, [sp, #176]

    mov x0, sp
    # 1 for is_exception
    mov x1, #1
    bl invoke_exception

    stp x0, x1, [sp, #176]
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x29, [sp, #144]
    ldp x30, xzr, [sp, #160]
    add sp, sp, #192
    eret

interrupt_entry:
   sub sp, sp, #192
   stp x0, x1, [sp, #0]
   stp x2, x3, [sp, #16]
   stp x4, x5, [sp, #32]
   stp x6, x7, [sp, #48]
   stp x8, x9, [sp, #64]
   stp x10, x11, [sp, #80]
   stp x12, x13, [sp, #96]
   stp x14, x15, [sp, #112]
   stp x16, x17, [sp, #128]
   stp x18, x29, [sp, #144]
   stp x30, xzr, [sp, #160]

   stp xzr, xzr, [sp, #176]

   mov x0, sp
   # 0 for is for is_exception
   mov x1, #0
   bl invoke_exception

   stp x0, x1, [sp, #176]
   ldp x0, x1, [sp, #0]
   ldp x2, x3, [sp, #16]
   ldp x4, x5, [sp, #32]
   ldp x6, x7, [sp, #48]
   ldp x8, x9, [sp, #64]
   ldp x10, x11, [sp, #80]
   ldp x12, x13, [sp, #96]
   ldp x14, x15, [sp, #112]
   ldp x16, x17, [sp, #128]
   ldp x18, x29, [sp, #144]
   ldp x30, xzr, [sp, #160]
   add sp, sp, #192
   eret

.balign 2048
exc_vector_table:
# the first 4 entries will never be reached
    b .
.balign 0x80
    b .
.balign 0x80
    b .
.balign 0x80
    b .
# Below exceptions happens inside the kernel spaces
# Synchronous Exception
.balign 0x80
    b exception_entry
# IRQ
.balign 0x80
    b interrupt_entry
# FIQ
.balign 0x80
    b interrupt_entry
# SError
.balign 0x80
    b exception_entry

"
);

#[no_mangle]
unsafe extern "C" fn invoke_exception(frame: *mut InterruptFrame, is_exception: bool) {
    unsafe {
        let f = if is_exception { exception } else { interrupt };
        f(&mut *frame)
    }
}

fn exception(frame: &mut InterruptFrame) {
    panic!("{}", frame);
}

fn interrupt(frame: &mut InterruptFrame) {
    _ = frame;
    todo!("interrupts")
}

#[inline(always)]
pub(super) fn init_exceptions() {
    let exc_vector_table: VirtAddr;
    unsafe {
        asm!("adr {0}, exc_vector_table", out(reg) exc_vector_table);
        crate::serial!("initializing the exception vector table at {exc_vector_table:#x}\n");
        asm!("msr VBAR_EL1, {0}", in(reg) exc_vector_table);
    }
}
