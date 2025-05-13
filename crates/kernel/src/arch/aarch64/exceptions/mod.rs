use core::{
    arch::{asm, global_asm},
    fmt::Display,
};

use crate::VirtAddr;

use super::registers::{Esr, GeneralPurpose};

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct InterruptFrame {
    // x0 ..= x28
    general_registers: [GeneralPurpose; 29],
    // x29
    fp: GeneralPurpose,
    // TODO: these aren't really general puropse
    elr: GeneralPurpose,
    spsr: GeneralPurpose,
    esr: Esr,
    far: GeneralPurpose,
    lr: GeneralPurpose,
    xzr: GeneralPurpose,
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
.equ CONTEXT_SIZE, 16 * 18
.macro EXCEPTION_VECTOR handler, save_eregs=0

    sub sp, sp, #CONTEXT_SIZE
# store general purpose registers
    stp x0, x1, [sp, #16 * 0]
    stp x2, x3, [sp, #16 * 1]
    stp x4, x5, [sp, #16 * 2]
    stp x6, x7, [sp, #16 * 3]
    stp x8, x9, [sp, #16 * 4]
    stp x10, x11, [sp, #16 * 5]
    stp x12, x13, [sp, #16 * 6]
    stp x14, x15, [sp, #16 * 7]
    stp x16, x17, [sp, #16 * 8]
    stp x18, x19, [sp, #16 * 9]
    stp x20, x21, [sp, #16 * 10]
    stp x22, x23, [sp, #16 * 11]
    stp x24, x25, [sp, #16 * 12]
    stp x26, x27, [sp, #16 * 13]
    stp x28, x29, [sp, #16 * 14]

    mrs x0, elr_el1
    mrs x1, spsr_el1
    stp x0, x1, [sp, #16 * 15]

    .if \\save_eregs
        mrs x0, esr_el1
        mrs x1, far_el1
        stp x0, x1, [sp, #16 * 16]
    .else
        stp xzr, xzr, [sp, #16 * 16]
    .endif

   # store link register which is x30
    stp x30, xzr, [sp, #16 * 17]
    mov x0, sp

# call exception handler
    bl \\handler
# avoid the 128 byte limit
    b exit_exception
.endm

.text
exit_exception:
# load elr and spsr before x0 and x1, these might be modified for example by context switching
    ldp x0, x1, [sp, #16 * 15]
    msr elr_el1, x0
    msr spsr_el1, x1

    ldp x0, x1, [sp, #16 * 0]
    ldp x2, x3, [sp, #16 * 1]
    ldp x4, x5, [sp, #16 * 2]
    ldp x6, x7, [sp, #16 * 3]
    ldp x8, x9, [sp, #16 * 4]
    ldp x10, x11, [sp, #16 * 5]
    ldp x12, x13, [sp, #16 * 6]
    ldp x14, x15, [sp, #16 * 7]
    ldp x16, x17, [sp, #16 * 8]
    ldp x18, x19, [sp, #16 * 9]
    ldp x20, x21, [sp, #16 * 10]
    ldp x22, x23, [sp, #16 * 11]
    ldp x24, x25, [sp, #16 * 12]
    ldp x26, x27, [sp, #16 * 13]
    ldp x28, x29, [sp, #16 * 14]
    # esr and far doesn't have to be restored
    ldp x30, xzr, [sp, #16 * 17]

    add sp, sp, #CONTEXT_SIZE
    eret

.global exc_vector_table
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
    EXCEPTION_VECTOR handle_sync_exception, 1
# IRQ
.balign 0x80
    EXCEPTION_VECTOR handle_irq, 0
# FIQ
.balign 0x80
    EXCEPTION_VECTOR handle_irq, 0
# SError
.balign 0x80
    EXCEPTION_VECTOR handle_serror, 1
"
);

#[no_mangle]
unsafe extern "C" fn handle_serror(frame: *mut InterruptFrame) {
    panic!("UNRECOVERABLE SERROR:\n{}", &*frame);
}

#[no_mangle]
unsafe extern "C" fn handle_sync_exception(frame: *mut InterruptFrame) {
    panic!("Synchronous Exception:\n{}", &*frame);
}

#[no_mangle]
unsafe extern "C" fn handle_irq(frame: *mut InterruptFrame) {
    interrupt(&mut *frame);
}

fn interrupt(frame: &mut InterruptFrame) {
    // TODO: figure out how to figure out the kind of interrupt
    super::timer::on_interrupt(frame);
}

#[inline(always)]
pub(super) fn init_exceptions() {
    let exc_vector_table: VirtAddr;
    unsafe {
        asm!("adr {0}, exc_vector_table", out(reg) exc_vector_table);
        asm!("msr VBAR_EL1, {0}", in(reg) exc_vector_table);
    }
}
