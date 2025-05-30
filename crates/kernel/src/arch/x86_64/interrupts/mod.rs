pub mod apic;
pub mod handlers;
mod idt;
mod pit;

use core::{arch::asm, fmt::Display};
use idt::IDTDesc;

use crate::{VirtAddr, KERNEL_ELF};

use super::threading::RFLAGS;

#[derive(Debug, Clone)]
#[repr(C)]
pub struct InterruptFrame {
    pub insturaction: VirtAddr,
    pub code_segment: u64,
    pub flags: RFLAGS,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

#[derive(Debug)]
#[repr(C)]
pub struct TrapFrame {
    pub error_code: u64,
    pub insturaction: VirtAddr,
    pub code_segment: u64,
    pub flags: RFLAGS,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

impl Display for TrapFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sym = KERNEL_ELF.sym_from_value_range(self.insturaction);

        let name = sym.map(|sym| KERNEL_ELF.string_table_index(sym.name_index).unwrap());
        let name = name.as_deref().unwrap_or("???");

        writeln!(f, "---- Trap Frame ----")?;
        writeln!(f, "at {:?} <{}>", self.insturaction, name)?;
        writeln!(
            f,
            "error code: {:#X}, rflags: {:#?}",
            self.error_code, self.flags
        )?;
        writeln!(f, "stack pointer: {:?}", self.stack_pointer)?;
        writeln!(
            f,
            "ss: {:#X}, cs: {:#X}",
            self.stack_segment, self.code_segment
        )?;

        Ok(())
    }
}

impl Display for InterruptFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sym = KERNEL_ELF.sym_from_value_range(self.insturaction);
        let name = sym.map(|sym| KERNEL_ELF.string_table_index(sym.name_index).unwrap());
        let name = name.as_deref().unwrap_or("???");

        writeln!(f, "---- Interrupt Frame ----")?;
        writeln!(f, "at {:?} <{}>", self.insturaction, name)?;
        writeln!(f, "rflags: {:#?}", self.flags)?;
        writeln!(f, "stack pointer: {:?}", self.stack_pointer)?;
        writeln!(
            f,
            "ss: {:#X}, cs: {:#X}",
            self.stack_segment, self.code_segment
        )?;

        Ok(())
    }
}

pub fn read_msr(msr: u32) -> usize {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "
            mov ecx, {0:e}
            rdmsr
            mov {1:e}, eax
            mov {2:e}, edx
            ",
            in(reg) msr, out(reg) low, out(reg) high
        );
    }

    (high as usize) << 32 | (low as usize)
}

pub fn init_idt() {
    unsafe {
        asm!("lidt [{}]", in(reg) &*IDTDesc, options(nostack));
    }
}
