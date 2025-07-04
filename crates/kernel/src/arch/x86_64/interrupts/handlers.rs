use super::super::syscalls::syscall_base;
use super::pit;
use core::arch::asm;
use core::cell::SyncUnsafeCell;
use lazy_static::lazy_static;

use super::idt::{GateDescriptor, IDTT};
use super::{InterruptFrame, TrapFrame};

use crate::arch::x86_64::interrupts::apic::send_eoi;
use crate::arch::x86_64::{inb, threading};
use crate::{drivers, serial};

pub const ATTR_TRAP: u8 = 0xF;
pub const ATTR_INT: u8 = 0xE;
const ATTR_RING3: u8 = 3 << 5;

const EMPTY_TABLE: IDTT = [GateDescriptor::default(); 256]; // making sure it is made at compile-time

macro_rules! create_idt {
    ($(($indx:literal, $handler:expr_2021, $attributes:expr_2021 $(, $ist:literal)?)),*) => {
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
        (3, breakpoint_handler, ATTR_INT | ATTR_RING3),
        (6, invaild_opcode, ATTR_INT),
        (8, dobule_fault_handler, ATTR_TRAP, 0),
        (0xC, stack_segment_fault_handler, ATTR_TRAP, 0),
        (13, general_protection_fault_handler, ATTR_TRAP),
        (14, page_fault_handler, ATTR_TRAP),
        (0x20, threading::context_switch_stub, ATTR_INT, 1),
        (0x21, keyboard_interrupt_handler, ATTR_INT),
        (0x22, pit::pit_handler, ATTR_INT),
        (0x80, syscall_base, ATTR_INT | ATTR_RING3),
        (0x81, do_nothing, ATTR_INT)
    );
}

#[unsafe(no_mangle)]
extern "x86-interrupt" fn divide_by_zero_handler(frame: InterruptFrame) {
    panic!("---- Divide By Zero Exception ----\n{}", frame);
}

extern "x86-interrupt" fn invaild_opcode(frame: InterruptFrame) {
    panic!("---- Invaild OPCODE ----\n{}", frame);
}

#[unsafe(no_mangle)]
extern "x86-interrupt" fn breakpoint_handler(frame: InterruptFrame) {
    serial!("hi from interrupt, breakpoint!\n{}", frame);
}

#[unsafe(no_mangle)]
extern "x86-interrupt" fn dobule_fault_handler(frame: TrapFrame) {
    panic!("---- Double Fault ----\n{}", frame);
}

#[unsafe(no_mangle)]
extern "x86-interrupt" fn stack_segment_fault_handler(frame: TrapFrame) {
    panic!("---- Stack-Segment Fault ----\n{}", frame);
}

#[unsafe(no_mangle)]
extern "x86-interrupt" fn general_protection_fault_handler(frame: TrapFrame) {
    panic!("---- General Protection Fault ----\n{}", frame,);
}

#[unsafe(no_mangle)]
extern "x86-interrupt" fn page_fault_handler(frame: TrapFrame) {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2) }

    panic!("---- Page Fault ----\naddress: {:#x}\n{}", cr2, frame)
}

#[inline]
pub fn handle_ps2_keyboard() {
    use drivers::keyboard::{set1::Set1Key, KEYBOARD};
    let key = inb(0x60);
    // outside of this function the keyboard should only be read from
    if let Some(encoded) = KEYBOARD
        .try_write()
        .map(|mut writer| writer.process_byte::<Set1Key>(key))
        .filter(|key| *key != drivers::keyboard::keys::Key::NULL_KEY)
    {
        crate::__navi_key_pressed(encoded);
    }
}
#[unsafe(no_mangle)]
pub extern "x86-interrupt" fn keyboard_interrupt_handler() {
    handle_ps2_keyboard();
    send_eoi();
}

#[unsafe(no_mangle)]
pub extern "x86-interrupt" fn do_nothing() {
    send_eoi();
}
