pub mod apic;
pub mod handlers;
mod idt;
mod pit;

use core::{arch::asm, fmt::Display};
use handlers::IDT;
use idt::IDTDesc;

use crate::{VirtAddr, KERNEL_ELF};

use super::threading::RFLAGS;
use crate::drivers::interrupts::IRQInfo;

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

const fn irq_handler<const IRQ_NUM: u32>() -> fn() {
    move || {
        let manager = crate::drivers::interrupts::IRQ_MANAGER.lock();
        for irq in &manager.irqs {
            if irq.irq_num == IRQ_NUM {
                irq.handler.handle_interrupt();
                apic::send_eoi();
                return;
            }
        }
    }
}

/// helper macro to count how many literals
macro_rules! count_idents {
    () => { 0 };
    ( $head:tt $(, $tail:tt)* ) => { 1 + count_idents!($($tail),*) };
}

/// A macro that both defines a `const IRQS` array and a `const HANDLERS` array
/// of `fn()`, one per IRQ.
///
/// - `irq_list!(3, 5, 7)` expands to:
///   ```rust
///   pub const IRQS: [usize; 3] = [3, 5, 7];
///   const HANDLERS: [fn(); 3] = [irq_handler::<3>(), irq_handler::<5>(), irq_handler::<7>()];
///   ```
macro_rules! irq_list {
    ( $( $x:literal ),* $(,)? ) => {
        /// A list of available System IRQ numbers (interrupt IDs) to use
        pub const IRQS: [u32; count_idents!($($x),*)] = [ $( $x ),* ];
        const HANDLERS: [fn(); count_idents!($($x),*)] = [ $( irq_handler::<$x>() ),* ];
    }
}

irq_list!(0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A);

/// Registers the handler function `handler` to irq `irq_num`
/// Make sure the num is retrieved from [`AVAILABLE_RQS`]
pub unsafe fn register_irq_handler(irq_num: u32, info: &IRQInfo) {
    let table = unsafe { &mut *IDT.get() };
    assert_eq!(table[irq_num as usize], idt::GateDescriptor::default());
    for (i, ava_irq) in IRQS.iter().enumerate() {
        if *ava_irq == irq_num {
            table[irq_num as usize] =
                idt::GateDescriptor::new(HANDLERS[i] as usize, handlers::ATTR_INT);
            return;
        }
    }
    panic!("IRQ {irq_num} not in irqs: {IRQS:?}");
}
