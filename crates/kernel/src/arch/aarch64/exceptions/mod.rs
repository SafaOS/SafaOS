use super::gic;
use crate::{arch::aarch64::timer::TIMER_IRQ, syscalls::syscall};
use core::{
    arch::{asm, global_asm},
    fmt::Display,
};

use super::registers::{Esr, ExcClass, Reg, Spsr};

global_asm!(include_str!("exceptions.s"));

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct InterruptFrame {
    // x0 ..= x28
    pub general_registers: [Reg; 29],
    // x29
    pub fp: Reg,
    // TODO: these aren't really general puropse
    pub elr: Reg,
    pub spsr: Spsr,
    pub esr: Esr,
    pub far: Reg,
    pub lr: Reg,
    /// The saved sp at the start of the interrupt (sp_el1)
    pub sp: Reg,
}

impl Display for InterruptFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Saved general purpose registers:")?;
        let cols = 3;
        let rows = self.general_registers.len() / cols;

        let write = |index: usize, f: &mut core::fmt::Formatter<'_>| {
            write!(f, "x{index}: {:x}    ", self.general_registers[index])
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

        let sp_el0: u64;
        unsafe {
            asm!("mrs {}, sp_el0", out(reg) sp_el0);
        }
        writeln!(f, "Special Registers:")?;
        writeln!(f, "FP: {:?}", self.fp)?;
        writeln!(f, "SP: {:?} (EL1)", self.sp)?;
        writeln!(f, "SP_EL0: {:?}", Reg(sp_el0))?;
        writeln!(f, "LR: {:?}", self.lr)?;
        writeln!(f, "SPSR: {:?}", self.spsr)?;
        writeln!(f, "ELR: {:#x}", self.elr)?;
        writeln!(f, "{}", self.esr)?;
        write!(f, "FAR: {:?}", self.far)?;
        Ok(())
    }
}

#[no_mangle]
unsafe extern "C" fn handle_serror(frame: *mut InterruptFrame) {
    panic!("UNRECOVERABLE SERROR:\n{}", &*frame);
}

#[no_mangle]
unsafe extern "C" fn handle_sync_exception(frame: *mut InterruptFrame) {
    unsafe {
        let frame = &mut *frame;
        exception(frame.esr.class(), frame)
    }
}

#[no_mangle]
unsafe extern "C" fn handle_irq(frame: *mut InterruptFrame) {
    unsafe {
        interrupt(&mut *frame);
    }
}

fn interrupt(frame: &mut InterruptFrame) {
    let int_id = gic::get_int_id();
    debug_assert!(
        int_id < 1020 || int_id > 1023,
        "FIXME: {int_id} is either an error or unimplemented and cannot be handled"
    );

    if int_id == TIMER_IRQ {
        super::timer::on_interrupt(frame);
    }
}

fn exception(kind: ExcClass, frame: &mut InterruptFrame) {
    match kind {
        ExcClass::SysCall => {
            let number = (*frame.esr & 0xFFFF) as u16;
            let registers = &mut frame.general_registers[0..7];
            let result: u16 = syscall(
                number,
                (*registers[0]) as usize,
                (*registers[1]) as usize,
                (*registers[2]) as usize,
                (*registers[3]) as usize,
                (*registers[4]) as usize,
            )
            .into();
            registers[0] = Reg(result as u64);
        }
        _ => panic!("Unhandled Synchronous Exception:\n{frame}"),
    }
}

#[inline(always)]
pub(super) fn init_exceptions() {
    let exc_vector_table: usize;
    unsafe {
        asm!("adr {0}, exc_vector_table", out(reg) exc_vector_table);
        asm!("msr VBAR_EL1, {0}", in(reg) exc_vector_table);
    }
}
