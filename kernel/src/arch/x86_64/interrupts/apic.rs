use core::{arch::asm, sync::atomic::AtomicUsize};

use super::{pit, read_msr};
use bitflags::bitflags;

use crate::{
    arch::{
        utils::{APIC_TIMER_TICKS_PER_20MS, TICKS_PER_20MS},
        x86_64::acpi::{self, MADT},
    },
    hddm, serial, PhysAddr, VirtAddr,
};

pub static IOAPIC_ADDR: AtomicUsize = AtomicUsize::new(0);

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct LVTEntry {
    pub entry: u8,
    pub flags: LVTEntryFlags,
    _res: u8,
}

impl LVTEntry {
    pub fn new(entry: u8, flags: LVTEntryFlags) -> Self {
        Self {
            entry,
            flags,
            _res: 0,
        }
    }
    pub fn encode_u32(self) -> u32 {
        unsafe { core::mem::transmute(self) }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct LVTEntryFlags: u16 {
        const LEVEL_TRIGGERED = 1 << 7;
        const DISABLED = 1 << 8;
        const TIMER_PERIODIC = 1 << 9;
    }
}

#[inline]
pub fn send_eoi() {
    unsafe {
        let address = get_local_apic_addr();
        let eoi_reg = get_local_apic_reg(address, 0xB0);
        let eoi_reg = eoi_reg as *mut u32;
        core::ptr::write_volatile(eoi_reg, 0)
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct MADTIOApic {
    _header: super::super::acpi::MADTRecord,
    pub ioapic_id: u8,
    _r: u8,
    pub ioapic_address: u32,
    global_system_interrupt_base: u32,
}

#[inline(always)]
pub fn get_io_apic_addr(madt: &MADT) -> VirtAddr {
    unsafe {
        let record = madt.get_record_of_type(1).unwrap() as *const MADTIOApic;
        let addr = (*record).ioapic_address as PhysAddr | hddm();
        addr
    }
}

#[inline(always)]
pub fn get_local_apic_addr() -> VirtAddr {
    let address = (read_msr(0x1B) & 0xFFFFF000) | hddm();

    address
}

#[inline(always)]
pub fn get_local_apic_reg(local_apic_addr: VirtAddr, local_apic_reg: u16) -> VirtAddr {
    local_apic_addr + local_apic_reg as usize
}

// NOTES:
// when we write the offset of the reg we want to access to ioregsel, iowin should have that reg
// no it is not the addr of that reg it is the reg itself each reg is 32bits long
pub unsafe fn write_ioapic_val_to_reg(ioapic_addr: VirtAddr, reg: u8, val: u32) {
    *(ioapic_addr as *mut u32) = reg as u32;
    *((ioapic_addr + 0x10) as *mut u32) = val;
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct IOREDTBL {
    pub entry: LVTEntry,
    _reserved: u16,
    _reserved1: u8,
    pub dest: u8,
}

impl IOREDTBL {
    pub const fn new(entry: LVTEntry, dest: u8) -> Self {
        Self {
            entry,
            _reserved: 0,
            _reserved1: 0,
            dest,
        }
    }

    pub const fn into_regs(self) -> (u32, u32) {
        let combined: u64 = unsafe { core::mem::transmute(self) };
        (combined as u32, (combined >> 31) as u32)
    }
}

pub unsafe fn write_ioapic_irq(n: u8, table: IOREDTBL) {
    let ioapic_addr = IOAPIC_ADDR.load(core::sync::atomic::Ordering::Relaxed) as VirtAddr;
    let offset1 = 0x10 + n * 2;
    let offset2 = offset1 + 1;

    let (lower, higher) = table.into_regs();

    write_ioapic_val_to_reg(ioapic_addr, offset1, lower);
    write_ioapic_val_to_reg(ioapic_addr, offset2, higher);
}

fn enable_apic_keyboard(apic_id: u8) {
    unsafe {
        let keyboard = IOREDTBL::new(LVTEntry::new(0x21, LVTEntryFlags::empty()), apic_id);

        write_ioapic_irq(1, keyboard);
    }
}

fn enable_apic_timer(local_apic_addr: VirtAddr, apic_id: u8) {
    fn apic_timer_ms_to_ticks(ms: u64) -> u32 {
        let ticks_per_ms =
            APIC_TIMER_TICKS_PER_20MS.load(core::sync::atomic::Ordering::Relaxed) / 20;
        (ms * ticks_per_ms) as u32
    }

    let addr = get_local_apic_reg(local_apic_addr, 0x320) as *mut u32;
    let init = get_local_apic_reg(local_apic_addr, 0x380) as *mut u32;
    let divide = get_local_apic_reg(local_apic_addr, 0x3E0) as *mut u8;
    let current_counter = get_local_apic_reg(local_apic_addr, 0x390) as *mut u32;

    // calibrate the timer
    unsafe {
        serial!("calibrating the apic timer\n");
        let timer = LVTEntry::new(0x81, LVTEntryFlags::empty());

        core::ptr::write_volatile(addr, timer.encode_u32());
        core::ptr::write_volatile(divide, 0x3);
        pit::prepare_sleep(20);
        asm!("sti");
        core::ptr::write_volatile(init, u32::MAX);

        let diff_tick = pit::calibrate_sleep(apic_id, || (), |()| u32::MAX - *current_counter);
        asm!("cli");

        APIC_TIMER_TICKS_PER_20MS.store(diff_tick as u64, core::sync::atomic::Ordering::Relaxed);
        serial!("calibrated with {} ticks in 20ms\n", diff_tick);
    }

    // enable the timer
    unsafe {
        let timer = LVTEntry::new(0x20, LVTEntryFlags::TIMER_PERIODIC);
        core::ptr::write_volatile(addr, timer.encode_u32());
        core::ptr::write_volatile(divide, 0x3);

        core::ptr::write_volatile(init, apic_timer_ms_to_ticks(10));
    }
}

pub fn calibrate_tsc(apic_id: u8) {
    serial!("calbrating tsc\n");
    unsafe {
        pit::prepare_sleep(20);

        asm!("sti");
        let diff_tick = pit::calibrate_sleep(
            apic_id,
            || core::arch::x86_64::_rdtsc(),
            |x| core::arch::x86_64::_rdtsc() - x,
        );
        asm!("cli");

        TICKS_PER_20MS.store(diff_tick, core::sync::atomic::Ordering::Relaxed);
        serial!("calbrated with {} ticks in 20ms\n", diff_tick);
    }
}

pub fn enable_apic_interrupts() {
    let local_apic_addr = get_local_apic_addr();
    let sivr = get_local_apic_reg(local_apic_addr, 0xF0) as *mut u32;

    unsafe {
        core::ptr::write_volatile(sivr, 0x1ff);

        let madt = MADT::get(acpi::get_sdt());
        let ioapic_addr = get_io_apic_addr(madt);
        IOAPIC_ADDR.store(ioapic_addr as usize, core::sync::atomic::Ordering::Relaxed);

        let apic_id = *(get_local_apic_reg(local_apic_addr, 0x20) as *const u8);
        calibrate_tsc(apic_id);
        enable_apic_timer(local_apic_addr, apic_id);
        enable_apic_keyboard(apic_id);
    }
}
