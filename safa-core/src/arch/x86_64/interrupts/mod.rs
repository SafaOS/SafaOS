pub mod apic;
pub mod handlers;
mod idt;
pub mod ps2;

use alloc::vec::Vec;
use core::{arch::asm, fmt::Display};
use handlers::IDT;
use idt::IDTDesc;

use crate::arch::x86_64::interrupts::apic::{APIC, IOREDTBL};
use crate::arch::x86_64::registers::RFLAGS;
use crate::utils::locks::Mutex;
use crate::{KERNEL_ELF, VirtAddr};

use crate::drivers::interrupts::{IRQInfo, IntTrigger};

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InterruptFrame {
    pub insturaction: VirtAddr,
    pub code_segment: u64,
    pub flags: RFLAGS,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

#[derive(Debug, Clone, Copy)]
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

        let name = sym.and_then(|sym| KERNEL_ELF.string_table_index(sym.name_index));
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

pub fn init_idt() {
    unsafe {
        asm!("lidt [{}]", in(reg) &*IDTDesc, options(nostack));
    }
}

const fn irq_handler<const IRQ_NUM: u32>() -> extern "x86-interrupt" fn(InterruptFrame) {
    extern "x86-interrupt" fn handler<const IRQ_NUM: u32>(_: InterruptFrame) {
        let manager = crate::drivers::interrupts::IRQ_MANAGER.read();
        for irq in &manager.irqs {
            if irq.irq_num == IRQ_NUM {
                irq.handler.handle_interrupt();
            }
        }

        apic::send_eoi();
    }
    return handler::<IRQ_NUM>;
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
        const HANDLERS: [extern "x86-interrupt" fn(InterruptFrame); count_idents!($($x),*)] = [ $( irq_handler::<$x>() ),* ];
    }
}

irq_list!(
    0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A
);

static NEXT_IRQ_NUM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0x90);

fn irq_num_to_index(irq_num: u32) -> usize {
    IRQS.iter()
        .position(|&x| x == irq_num)
        .expect("IRQ number not found in list of available IRQs")
}

fn allocate_next_irq() -> u32 {
    let allocated = NEXT_IRQ_NUM.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

    let table = unsafe { &mut *IDT.get() };
    assert_eq!(table[allocated as usize], idt::GateDescriptor::default());
    allocated
}

static REGISTERED_PCI_IRQS: Mutex<Vec<(u8, u32)>> = Mutex::new(Vec::new());

/// Registers the handler function `handler` to irq `irq_num`
pub unsafe fn register_irq_handler(info: &IRQInfo, triggering: IntTrigger) -> u32 {
    let (irq_num, is_new) = match info {
        // No additional setup needed.
        IRQInfo::MSIX(_) | IRQInfo::MSI(_) => (allocate_next_irq(), true),
        IRQInfo::PCIInt {
            interrupt_line,
            interrupt_pin,
            bus,
            device,
            function,
        } => unsafe {
            _ = bus;
            _ = device;
            _ = function;
            _ = interrupt_pin;
            let mut registered_pci_irq = REGISTERED_PCI_IRQS.lock();

            if let Some((_, irq_num)) = registered_pci_irq
                .iter()
                .find(|(i, _)| *i == *interrupt_line)
            {
                (*irq_num, false)
            } else {
                let irq_num = allocate_next_irq();
                let redirection = IOREDTBL::new()
                    .with_vector(irq_num as u8)
                    .with_level_triggered(
                        triggering == IntTrigger::LevelAssert
                            || triggering == IntTrigger::LevelDeassert,
                    )
                    .with_masked(false);

                crate::serial!("Registering: {irq_num}, tbl: {redirection:#?}\n");
                APIC.write_ioapic_irq(*interrupt_line, redirection);
                crate::serial!("Wrote!\n");
                registered_pci_irq.push((*interrupt_line, irq_num));
                (irq_num, true)
            }
        },
    };

    let irq_index = irq_num_to_index(irq_num);
    if is_new {
        let table = unsafe { &mut *IDT.get() };
        table[irq_num as usize] =
            idt::GateDescriptor::new(HANDLERS[irq_index] as usize, handlers::ATTR_INT);
    }
    irq_num
}
