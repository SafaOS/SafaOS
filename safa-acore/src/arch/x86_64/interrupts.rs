use core::cell::SyncUnsafeCell;

use crate::{arch::x86_64::registers::CapturedCpuStatus, logging, misc::VirtAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, packed)]
pub struct GateDescriptor {
    offset0: u16,
    selector: u16,
    ist: u8,
    attributes: u8, // gate_type, dpl, zero and present bit
    offset1: u16,
    offset2: u32,
    reserved: u32,
}

impl GateDescriptor {
    pub const fn new(handler: usize, attributes: u8) -> Self {
        let offset = handler;
        Self {
            offset0: offset as u16,
            selector: 0x08,
            ist: 0,
            attributes: attributes | 1 << 7, // attaching present attriubute
            offset1: (offset >> 16) as u16,
            offset2: (offset >> 32) as u32,
            reserved: 0,
        }
    }

    pub const fn default() -> Self {
        Self {
            offset0: 0,
            selector: 0,
            ist: 0,
            attributes: 0,
            offset1: 0,
            offset2: 0,
            reserved: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
/// Interrupt's allowed Ring
pub enum Ring {
    /// Ring 0
    Kernel = 0,
    /// Ring 3
    Userspace = 3,
}

#[derive(Debug, Clone, Copy)]
pub enum GateKind {
    /// A Trap is ran with interrupts enabled for fast interrupt handling unlink [`Self::Int`].
    Trap = 0xF,
    /// An interrupt gate is ran without interrupts enabled.
    Int = 0xE,
}

pub type InterruptEntry = extern "C" fn(CapturedCpuStatus);
pub type RawInterruptHandler = unsafe extern "C" fn() -> !;

/// Turns an [`InterruptEntry`] into a [`RawInterruptHandler`].
macro_rules! make_handler {
    (int $name: ident) => {
        {
            #[unsafe(naked)]
            unsafe extern "C" fn wrapper() -> ! {
                const _: $crate::arch::x86_64::interrupts::InterruptEntry = $name;

                core::arch::naked_asm!(
                            "
                        sub rsp, 8
                        push rbp
                        mov rbp, cr3
                        push rbp
                        push r15
                        push r14
                        push r13
                        push r12
                        push r11
                        push r10
                        push r9
                        push r8
                        push rsi
                        push rdi
                        push rdx
                        push rcx
                        push rbx
                        push rax
                        cld

                        call {0}

                        pop rax
                        pop rbx
                        pop rcx
                        pop rdx
                        pop rdi
                        pop rsi
                        pop r8
                        pop r9
                        pop r10
                        pop r11
                        pop r12
                        pop r13
                        pop r14
                        pop r15

                        // cr3
                        pop rbp
                        pop rbp
                        // pop 0
                        add rsp, 8
                        iretq
                        ud2
                        ", sym $name
                        )

            }

            wrapper
        }
    };
    (exc $name: ident) => {
        {
            #[unsafe(naked)]
            unsafe extern "C" fn wrapper() -> ! {
                const _: $crate::arch::x86_64::interrupts::InterruptEntry = $name;

                core::arch::naked_asm!(
                            "
                        push rbp
                        mov rbp, cr3
                        push rbp
                        push r15
                        push r14
                        push r13
                        push r12
                        push r11
                        push r10
                        push r9
                        push r8
                        push rsi
                        push rdi
                        push rdx
                        push rcx
                        push rbx
                        push rax
                        cld

                        call {0}

                        pop rax
                        pop rbx
                        pop rcx
                        pop rdx
                        pop rdi
                        pop rsi
                        pop r8
                        pop r9
                        pop r10
                        pop r11
                        pop r12
                        pop r13
                        pop r14
                        pop r15

                        // cr3
                        pop rbp
                        pop rbp
                        iretq
                        ud2
                        ", sym $name
                        )

            }

            wrapper
        }
    };
}

/// The IDT
pub struct InterruptTable([GateDescriptor; 256]);

impl InterruptTable {
    pub const fn new() -> Self {
        Self([GateDescriptor::default(); 256])
    }

    fn gate_inner(
        mut self,
        index: usize,
        entry: RawInterruptHandler,
        kind: GateKind,
        ring: Ring,
        ist: Option<u8>,
    ) -> Self {
        self.0[index] = GateDescriptor::new(entry as usize, kind as u8 | (ring as u8) << 5);
        if let Some(ist) = ist {
            self.0[index].ist = ist;
        }
        self
    }

    pub fn gate(
        self,
        index: usize,
        entry: RawInterruptHandler,
        kind: GateKind,
        ring: Ring,
    ) -> Self {
        self.gate_inner(index, entry, kind, ring, None)
    }

    fn gate_with_ist(
        self,
        index: usize,
        entry: RawInterruptHandler,
        kind: GateKind,
        ring: Ring,
        ist: u8,
    ) -> Self {
        self.gate_inner(index, entry, kind, ring, Some(ist))
    }
}

static IDT: SyncUnsafeCell<InterruptTable> = SyncUnsafeCell::new(InterruptTable::new());

#[repr(C, packed)]
pub struct IDTDescriptor {
    limit: u16,
    base: usize,
}

static IDTR: SyncUnsafeCell<IDTDescriptor> =
    SyncUnsafeCell::new(IDTDescriptor { limit: 0, base: 0 });

/// Returns the default IDT
fn system_idt() -> InterruptTable {
    InterruptTable::new()
        .gate(
            3,
            make_handler!(int breakpoint_handler),
            GateKind::Trap,
            Ring::Userspace,
        )
        .gate_with_ist(
            8,
            make_handler!(exc double_fault_handler),
            GateKind::Int,
            Ring::Kernel,
            0,
        )
        .gate(
            13,
            make_handler!(exc gpf_handler),
            GateKind::Trap,
            Ring::Kernel,
        )
        .gate_with_ist(
            14,
            make_handler!(exc page_fault_handler),
            GateKind::Trap,
            Ring::Kernel,
            2,
        )
}

/// Initializes the IDT for the given CPU
///
/// Safety: init_idt(bsp = true) must only be called once from the BSP on init.
pub unsafe fn init_idt(bsp: bool) {
    let r = IDT.get();
    let d_r = IDTR.get();
    if bsp {
        unsafe {
            let sys_idt = system_idt();
            let idt_limit = sys_idt.0.len() as u16;

            r.write_volatile(sys_idt);
            d_r.write_volatile(IDTDescriptor {
                limit: idt_limit,
                base: r as *mut InterruptTable as usize,
            });
        }
    }

    unsafe {
        core::arch::asm!("lidt [{}]", in(reg) d_r);
    }
}

#[unsafe(no_mangle)]
extern "C" fn page_fault_handler(ctx: CapturedCpuStatus) {
    let cr2: usize;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nostack, nomem, preserves_flags));
    }

    let addr = VirtAddr::new(cr2);
    panic!(
        "=== Page Fault {addr:?}, error: {:#x} ===:\n{ctx}",
        ctx.error_code()
    )
}

#[unsafe(no_mangle)]
extern "C" fn gpf_handler(ctx: CapturedCpuStatus) {
    panic!(
        "=== General Protection Fault, error: {:#x} ===:\n{ctx}",
        ctx.error_code()
    )
}

#[unsafe(no_mangle)]
extern "C" fn double_fault_handler(ctx: CapturedCpuStatus) {
    panic!(
        "=== Double Fault, error: {:#x} ===:\n{ctx}",
        ctx.error_code()
    )
}

#[unsafe(no_mangle)]
extern "C" fn breakpoint_handler(ctx: CapturedCpuStatus) {
    logging::trace!("interrupt", "Breakpoint: {ctx}");
}
