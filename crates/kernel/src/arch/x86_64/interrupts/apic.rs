use super::{pit, read_msr};
use crate::{
    arch::{
        paging::PageTable,
        x86_64::{
            acpi,
            utils::{APIC_TIMER_TICKS_PER_MS, TICKS_PER_MS},
        },
    },
    info,
    memory::paging::{EntryFlags, MapToError},
    serial, PhysAddr, VirtAddr,
};
use bitflags::bitflags;
use core::arch::asm;
use lazy_static::lazy_static;

/// Maps the IOAPIC and the Local APIC to the `dest` page table
pub unsafe fn map_apic(dest: &mut PageTable) -> Result<(), MapToError> {
    let flags = EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE;
    let lapic_phys = *LAPIC_PHYS_ADDR;
    let lapic_virt = lapic_phys.into_virt();
    let ioapic_phys = *IOAPIC_PHYS_ADDR;
    let ioapic_virt = ioapic_phys.into_virt();

    unsafe {
        dest.map_contiguous_pages(lapic_virt, lapic_phys, 1, flags)?;
        dest.map_contiguous_pages(ioapic_virt, ioapic_phys, 1, flags)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct LVTEntry {
    entry: u8,
    flags: LVTEntryFlags,
}

impl LVTEntry {
    pub const fn new(entry: u8, flags: LVTEntryFlags) -> Self {
        Self { entry, flags }
    }

    pub const fn encode_u32(self) -> u32 {
        self.entry as u32 | ((self.flags.bits() as u32) << 8)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct LVTEntryFlags: u16 {
        const DISABLED = 1 << 8;
        const TIMER_PERIODIC = 1 << 9;
        const TSC_DEADLINE = 2 << 9;
    }
}

#[inline]
pub fn send_eoi() {
    unsafe {
        let address = get_local_apic_addr();
        let eoi_reg = get_local_apic_reg(address, 0xB0);
        let eoi_reg = eoi_reg.into_ptr::<u32>();
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
pub fn get_io_apic_addr() -> VirtAddr {
    *IOAPIC_ADDR
}

lazy_static! {
    pub static ref LAPIC_PHYS_ADDR: PhysAddr = {
        let phys = read_msr(0x1B) & 0xFFFFF000;
        let phys = PhysAddr::from(phys);
        phys
    };
    static ref LAPIC_ADDR: VirtAddr = LAPIC_PHYS_ADDR.into_virt();
    pub static ref LAPIC_ID: u8 =
        unsafe { ((*(get_local_apic_reg(*LAPIC_ADDR, 0x20).into_ptr::<u32>())) >> 24) as u8 };
    pub static ref IOAPIC_PHYS_ADDR: PhysAddr = unsafe {
        let madt = *acpi::MADT_DESC;
        let record = madt.get_record_of_type(1).unwrap() as *const MADTIOApic;

        let addr = PhysAddr::from((*record).ioapic_address as usize);
        addr
    };
    static ref IOAPIC_ADDR: VirtAddr = IOAPIC_PHYS_ADDR.into_virt();
}
#[inline(always)]
pub fn get_local_apic_addr() -> VirtAddr {
    *LAPIC_ADDR
}

#[inline(always)]
pub fn get_local_apic_reg(local_apic_addr: VirtAddr, local_apic_reg: u16) -> VirtAddr {
    local_apic_addr + local_apic_reg as usize
}

// NOTES:
// when we write the offset of the reg we want to access to ioregsel, iowin should have that reg
// no it is not the addr of that reg it is the reg itself each reg is 32bits long
pub unsafe fn write_ioapic_val_to_reg(ioapic_addr: VirtAddr, reg: u8, val: u32) {
    let reg_addr = ioapic_addr.into_ptr::<u32>();
    let val_addr = (ioapic_addr + 0x10).into_ptr::<u32>();

    core::ptr::write_volatile(reg_addr, reg as u32);
    core::ptr::write_volatile(val_addr, val);
}

#[derive(Debug, Clone, Copy)]
pub struct IOREDTBL {
    entry: LVTEntry,
    dest: u8,
}

impl IOREDTBL {
    pub const fn new(entry: LVTEntry, dest: u8) -> Self {
        Self { entry, dest }
    }

    pub const fn into_regs(self) -> (u32, u32) {
        let as_u64 = self.entry.encode_u32() as u64 | ((self.dest as u64) << 56);
        (as_u64 as u32, (as_u64 >> 31) as u32)
    }
}

pub unsafe fn write_ioapic_irq(n: u8, table: IOREDTBL) {
    let ioapic_addr = get_io_apic_addr();
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

        info!("enabled APIC Keyboard.");
    }
}

fn enable_apic_timer(local_apic_addr: VirtAddr, apic_id: u8) {
    info!("enabling apic timer...");
    fn apic_timer_ms_to_ticks(ms: u64) -> u32 {
        let ticks_per_ms = unsafe { core::ptr::read(APIC_TIMER_TICKS_PER_MS.get()) };
        (ms * ticks_per_ms) as u32
    }

    let addr = get_local_apic_reg(local_apic_addr, 0x320).into_ptr::<u32>();
    let init = get_local_apic_reg(local_apic_addr, 0x380).into_ptr::<u32>();
    let divide = get_local_apic_reg(local_apic_addr, 0x3E0).into_ptr::<u8>();
    let current_counter = get_local_apic_reg(local_apic_addr, 0x390).into_ptr::<u32>();

    // calibrate the timer
    unsafe {
        serial!("calibrating the apic timer\n");
        let timer = LVTEntry::new(0x81, LVTEntryFlags::empty());

        core::ptr::write_volatile(addr, timer.encode_u32());
        core::ptr::write_volatile(divide, 0x3);
        pit::prepare_sleep(100);
        asm!("sti");
        core::ptr::write_volatile(init, u32::MAX);

        let diff_tick = pit::calibrate_sleep(apic_id, || (), |()| u32::MAX - *current_counter);
        asm!("cli");

        core::ptr::write_volatile(APIC_TIMER_TICKS_PER_MS.get(), diff_tick as u64 / 100);
        info!(
            "APIC Timer calibrated with {} ticks in 100ms",
            core::ptr::read(APIC_TIMER_TICKS_PER_MS.get()) * 100
        );
    }

    // enable the timer
    unsafe {
        let timer = LVTEntry::new(0x20, LVTEntryFlags::TIMER_PERIODIC);
        core::ptr::write_volatile(addr, timer.encode_u32());
        core::ptr::write_volatile(divide, 0x3);

        core::ptr::write_volatile(init, apic_timer_ms_to_ticks(5));
    }
}

pub fn calibrate_tsc(apic_id: u8) {
    serial!("calbrating tsc\n");
    unsafe {
        pit::prepare_sleep(100);

        asm!("sti");
        let diff_tick = pit::calibrate_sleep(
            apic_id,
            || core::arch::x86_64::_rdtsc(),
            |x| core::arch::x86_64::_rdtsc() - x,
        );
        asm!("cli");

        core::ptr::write_volatile(TICKS_PER_MS.get(), diff_tick / 100);
        info!(
            "calibrated TSC with {} ticks in 100ms",
            core::ptr::read(TICKS_PER_MS.get()) * 100
        );
    }
}

pub fn enable_apic_interrupts() {
    let local_apic_addr = get_local_apic_addr();
    let sivr = get_local_apic_reg(local_apic_addr, 0xF0).into_ptr::<u32>();

    unsafe {
        core::ptr::write_volatile(sivr, 0x1ff);

        let ioapic_addr = get_io_apic_addr();

        let apic_id = *LAPIC_ID;
        info!("enabled APIC, apic_id is {apic_id}, IO APIC is at {ioapic_addr:#x}, local APIC is at {local_apic_addr:#x}");
        calibrate_tsc(apic_id);
        enable_apic_timer(local_apic_addr, apic_id);
        enable_apic_keyboard(apic_id);
    }
}
