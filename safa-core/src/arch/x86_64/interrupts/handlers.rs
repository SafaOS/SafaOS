use super::super::syscalls::syscall_base;
use core::arch::asm;
use core::cell::SyncUnsafeCell;
use core::fmt::Display;
use core::sync::atomic::{AtomicUsize, Ordering};
use lazy_static::lazy_static;

use super::idt::{GateDescriptor, IDTT};
use super::{InterruptFrame, TrapFrame};

use crate::arch::threading::CPUStatus;
use crate::arch::x86_64::interrupts::apic::send_eoi;
use crate::arch::x86_64::interrupts::ps2::{self};
use crate::arch::x86_64::{threading, tlb};
use crate::{khalt, serial};

pub static NMI_REASON: AtomicUsize = AtomicUsize::new(0);

pub const HALT_ALL_NMI: usize = 0x23;
pub const TLBI_ID: u8 = 0x24;
pub const APIC_ERROR_HANDLER_ID: u8 = 0x25;
pub const MOUSE_HANDLER_ID: u8 = 0x26;

pub const ATTR_TRAP: u8 = 0xF;
pub const ATTR_INT: u8 = 0xE;
const ATTR_RING3: u8 = 3 << 5;

#[repr(C)]
pub struct InterruptCpuFrame {
    pub capture: CPUStatus,
    pub error_code: u64,
}

impl Display for InterruptCpuFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "--------- CPU Frame: ---------\n{}\nerror code: {:#x}",
            self.capture, self.error_code
        )
    }
}

macro_rules! make_handler {
    ($name:path) => {{
        #[unsafe(naked)]
        extern "C" fn wrapper() -> ! {
                const _: extern "C" fn (&mut InterruptCpuFrame) = $name;

                core::arch::naked_asm!(
                    "
                and rsp, -16        // alignment for the interrupt frame
                sub rsp, 512      // allocate space for fpu registers
                fxsave [rsp]

                /* alignment */
                push rax

                push rax
                mov rax, cr3
                push rax

                push rbx
                push rcx
                push rdx

                push rsi
                push rdi
                push rbp

                push r8
                push r9
                push r10
                push r11
                push r12
                push r13
                push r14
                push r15

                // iretq frame
                lea rax, [rsp+(0x2C0-(8*7))]
                push [rax+0x08]     // rip
                push [rax+0x10]    // cs
                push [rax+0x28]   // ss
                push [rax+0x18]  // rflags
                push [rax+0x20] // rsp
                // ring0 rsp
                push 0
                // fs
                push 0
                cld

                mov rdi, rsp
                call {0}

                mov rdi, rsp
                call restore_cpu_status_partial
                ud2
                ", sym $name
                )

        }

        wrapper
    }};
}

const EMPTY_TABLE: IDTT = [GateDescriptor::default(); 256]; // making sure it is made at compile-time
macro_rules! create_idt {
    ($(($indx:expr, $handler:expr_2021, $attributes:expr_2021 $(, $ist:literal)?)),*) => {
        {
            let mut table = EMPTY_TABLE;
            $(
                let index: usize = $indx as usize;
                let handler: usize = $handler as usize;
                let attributes: u8 = $attributes;
                let ist: u8 = {
                    #[allow(unused_variables)]
                    let ist_value: i8 = -1;
                    $(let ist_value = $ist as i8;)?
                    (ist_value + 1) as u8
                };
                table[index] = GateDescriptor::new(handler, attributes);
                table[index].ist = ist;
            )*
            SyncUnsafeCell::new(table)
        }
    };
}

lazy_static! {
    pub static ref IDT: SyncUnsafeCell<IDTT> = create_idt!(
        (0, divide_by_zero_handler, ATTR_INT),
        (2, nmi_handler, ATTR_INT),
        (3, breakpoint_handler, ATTR_INT | ATTR_RING3),
        (6, invalid_opcode, ATTR_INT),
        (8, double_fault_handler, ATTR_TRAP, 0),
        (0xC, stack_segment_fault_handler, ATTR_TRAP, 0),
        (13, make_handler!(general_protection_fault), ATTR_TRAP),
        (14, make_handler!(page_fault), ATTR_TRAP, 0),
        (0x13, simd_exception_handler, ATTR_TRAP),
        (
            0x20,
            make_handler!(threading::context_switch_on_int),
            ATTR_INT,
            1
        ),
        (0x21, keyboard_interrupt_handler, ATTR_INT),
        (TLBI_ID, tlb::tlbi_flush_handler, ATTR_INT),
        (APIC_ERROR_HANDLER_ID, apic_err, ATTR_INT),
        (MOUSE_HANDLER_ID, mice_handler, ATTR_INT),
        (0x80, syscall_base, ATTR_INT | ATTR_RING3),
        (0x81, do_nothing, ATTR_INT)
    );
}

extern "x86-interrupt" fn nmi_handler(_: InterruptFrame) {
    match NMI_REASON.load(Ordering::Relaxed) {
        HALT_ALL_NMI => halt_handler(),
        r => panic!("Unknown NMI {r}"),
    }
}
extern "x86-interrupt" fn apic_err(_: InterruptFrame) {
    panic!("APIC error encountured")
}

pub static HALTED_CPUS: AtomicUsize = AtomicUsize::new(0);
fn halt_handler() -> ! {
    HALTED_CPUS.fetch_add(1, Ordering::SeqCst);
    crate::serial!("halting...\n");
    send_eoi();
    khalt()
}

extern "x86-interrupt" fn divide_by_zero_handler(frame: InterruptFrame) {
    panic!("---- Divide By Zero Exception ----\n{}", frame);
}

extern "x86-interrupt" fn invalid_opcode(frame: InterruptFrame) {
    panic!("---- Invalid OPCODE ----\n{}", frame);
}

extern "x86-interrupt" fn simd_exception_handler(frame: InterruptFrame) {
    // TODO: print mxcsr
    panic!("---- SIMD Exception ----\n{}", frame);
}

extern "x86-interrupt" fn breakpoint_handler(frame: InterruptFrame) {
    serial!("hi from interrupt, breakpoint!\n{}", frame);
}

extern "x86-interrupt" fn double_fault_handler(frame: TrapFrame) {
    panic!("---- Double Fault ----\n{}", frame);
}

extern "x86-interrupt" fn stack_segment_fault_handler(frame: TrapFrame) {
    panic!("---- Stack-Segment Fault ----\n{}", frame);
}

extern "C" fn general_protection_fault(frame: &mut InterruptCpuFrame) {
    panic!("---- General Protection Fault ----\n{}", frame);
}

extern "C" fn page_fault(frame: &mut InterruptCpuFrame) {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2) }

    panic!("---- Page Fault ----\naddress: {:#x}\n{}", cr2, frame)
}

pub extern "x86-interrupt" fn keyboard_interrupt_handler(_: InterruptFrame) {
    ps2::handle_ps2_keyboard();
    send_eoi();
}

pub extern "x86-interrupt" fn mice_handler(_: InterruptFrame) {
    ps2::mice_handler();
    send_eoi();
}

pub extern "x86-interrupt" fn do_nothing(_: InterruptFrame) {
    send_eoi();
}
