use super::pit;
use crate::{
    PhysAddr, VirtAddr,
    arch::{
        paging::PageTable,
        x86_64::{
            acpi,
            interrupts::handlers::APIC_ERROR_HANDLER_ID,
            outb,
            registers::{rdmsr, wrmsr},
            utils::APIC_TIMER_TICKS_PER_MS,
        },
    },
    info,
    memory::paging::{EntryFlags, MapToError},
    serial,
    utils::locks::SpinLock,
};
use bitfield_struct::bitfield;
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

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum APICDeliveryMode {
    Fixed = 0,
    LowestPiriority = 1,
    SMI = 0b010,
    Reserved = 0b011,
    NMI = 0b100,
    INIT = 0b101,
    StartUp = 0b110,
    Reserved2 = 0b111,
}

impl APICDeliveryMode {
    pub const fn from_bits(bits: u8) -> Self {
        assert!((bits & !(0xF)) == 0);
        unsafe { core::mem::transmute(bits) }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum APICDestShorthand {
    NoShortHand = 0,
    SelfOnly = 1,
    All = 2,
    ExcludingSelf = 3,
}

impl APICDestShorthand {
    pub const fn from_bits(bits: u8) -> Self {
        assert!((bits & !(0b11)) == 0);
        unsafe { core::mem::transmute(bits) }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

#[bitfield(u64)]
pub struct APICICReg {
    vector: u8,
    #[bits(3)]
    delivery_mode: APICDeliveryMode,
    /// Destintion mode
    ///
    /// 0 == Physical
    /// 1 == Logical
    dest_logical: bool,
    /// Delivery Mode
    /// 0 == Idle
    ///
    /// 1 == Send Pending
    delivery_send_pending: bool,
    #[bits(1)]
    __: (),
    /// Level
    ///
    /// 0 == De-assert
    ///
    /// 1 == Assert
    assert: bool,
    /// Trigger Mode
    ///
    /// 0 == Edge Triggered
    ///
    /// 1 == Level TRiggered
    level_triggered: bool,
    #[bits(2)]
    __: (),
    #[bits(2)]
    destination_shorthand: APICDestShorthand,
    #[bits(36)]
    __: (),
    destination_field: u8,
}

pub fn write_ic_reg(value: APICICReg) {
    let lapic_addr = get_lapic_addr();
    let low = get_lapic_reg(lapic_addr, 0x300);
    let high = get_lapic_reg(lapic_addr, 0x310);

    let value_bits = value.into_bits();
    let (value_low, value_high) = (value_bits as u32, (value_bits >> 32) as u32);
    unsafe {
        low.write_volatile(value_low);
        high.write_volatile(value_high);
    }
}

pub fn read_ic_reg() -> APICICReg {
    let lapic_addr = get_lapic_addr();
    let low = get_lapic_reg(lapic_addr, 0x300);
    let high = get_lapic_reg(lapic_addr, 0x310);
    unsafe {
        let low_bits = low.read_volatile();
        let high_bits = high.read_volatile();
        let bits: u64 = low_bits as u64 | (high_bits as u64) << 32;
        APICICReg::from_bits(bits)
    }
}

/// Sends an NMI to all processors
pub fn send_nmi_all(vector: u8) {
    write_ic_reg(
        APICICReg::new()
            .with_destination_shorthand(APICDestShorthand::ExcludingSelf)
            .with_vector(vector),
    );

    while read_ic_reg().delivery_send_pending() {
        core::hint::spin_loop();
    }
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
        let address = get_lapic_addr();
        let eoi_reg = get_lapic_reg(address, 0xB0);
        eoi_reg.write_volatile(0);
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
        let phys = rdmsr(0x1B) & 0xFFFFF000;
        let phys = PhysAddr::from(phys);
        phys
    };
    static ref LAPIC_ADDR: VirtAddr = LAPIC_PHYS_ADDR.into_virt();
    pub static ref IOAPIC_PHYS_ADDR: PhysAddr = unsafe {
        let madt = *acpi::MADT_DESC;
        let record = madt.get_record_of_type(1).unwrap() as *const MADTIOApic;

        let addr = PhysAddr::from((*record).ioapic_address as usize);
        addr
    };
    static ref IOAPIC_ADDR: VirtAddr = IOAPIC_PHYS_ADDR.into_virt();
}

#[inline(always)]
pub fn get_lapic_addr() -> VirtAddr {
    *LAPIC_ADDR
}

#[inline(always)]
const fn get_lapic_reg_addr(lapic_addr: VirtAddr, lapic_reg: u16) -> VirtAddr {
    lapic_addr + lapic_reg as usize
}
#[inline(always)]
fn get_lapic_reg(lapic_addr: VirtAddr, lapic_off: u16) -> *mut u32 {
    get_lapic_reg_addr(lapic_addr, lapic_off).into_ptr::<u32>()
}

#[inline(always)]
fn read_lapic_reg(lapic_addr: VirtAddr, lapic_off: u16) -> u32 {
    unsafe {
        // performs a dword read as expected from the local APIC
        get_lapic_reg(lapic_addr, lapic_off).read_volatile()
    }
}

#[inline(always)]
pub fn get_lapic_id(lapic_addr: VirtAddr) -> u8 {
    (read_lapic_reg(lapic_addr, 0x20) >> 24) as u8
}

pub unsafe fn write_ioapic_val_to_reg(ioapic_addr: VirtAddr, reg: u8, val: u32) {
    unsafe {
        let ioregsel_addr = ioapic_addr.into_ptr::<u32>();
        let iowin_addr = (ioapic_addr + 0x10).into_ptr::<u32>();

        core::ptr::write_volatile(ioregsel_addr, reg as u32);
        core::ptr::write_volatile(iowin_addr, val);
    }
}

pub unsafe fn read_ioapic_reg(ioapic_addr: VirtAddr, reg: u8) -> u32 {
    unsafe {
        let ioregsel_addr = ioapic_addr.into_ptr::<u32>();
        let iowin_addr = (ioapic_addr + 0x10).into_ptr::<u32>();

        core::ptr::write_volatile(ioregsel_addr, reg as u32);
        core::ptr::read_volatile(iowin_addr)
    }
}

pub fn get_ioapic_id(ioapic_addr: VirtAddr) -> u8 {
    unsafe { (read_ioapic_reg(ioapic_addr, 0) >> 24) as u8 }
}

#[bitfield(u64)]
pub struct IOREDTBL {
    pub(super) vector: u8,
    #[bits(3)]
    pub(super) delivery_mode: APICDeliveryMode,
    pub(super) destination_logical: bool,
    delivery_pending: bool,
    pin_polarity_active_low: bool,
    remote_irr: bool,
    /// Otherwise edge triggered
    level_triggered: bool,
    pub(super) masked: bool,

    timer_perodic: bool,
    tsc_deadline: bool,
    #[bits(37)]
    __: (),
    pub(super) destination: u8,
}

impl IOREDTBL {
    pub const fn into_regs(self) -> (u32, u32) {
        let as_u64 = self.into_bits();
        (as_u64 as u32, (as_u64 >> 32) as u32)
    }
}

pub unsafe fn write_ioapic_irq(n: u8, table: IOREDTBL) {
    unsafe {
        let ioapic_addr = get_io_apic_addr();
        let offset1 = 0x10 + (n * 2);
        let offset2 = offset1 + 1;

        let (lower, higher) = table.into_regs();
        write_ioapic_val_to_reg(ioapic_addr, offset1, lower);
        write_ioapic_val_to_reg(ioapic_addr, offset2, higher);
    }
}

pub fn enable_apic_keyboard() {
    unsafe {
        let lapic_addr = get_lapic_addr();
        let lapic_id = get_lapic_id(lapic_addr);

        let keyboard = IOREDTBL::new().with_vector(0x21).with_destination(lapic_id);
        write_ioapic_irq(1, keyboard);

        info!("enabled APIC Keyboard.");
    }
}

fn configure_error(lapic_addr: VirtAddr) {
    let addr = get_lapic_reg(lapic_addr, 0x370);
    let entry = LVTEntry::new(APIC_ERROR_HANDLER_ID, LVTEntryFlags::empty());
    unsafe {
        addr.write_volatile(entry.encode_u32());
    }
}

fn enable_apic_timer(local_apic_addr: VirtAddr, lapic_id: u8) {
    static _CALIBRATE_LOCK: SpinLock<()> = SpinLock::new(());
    let _guard = _CALIBRATE_LOCK.lock();

    info!("enabling apic timer for lapic: {lapic_id}...");
    fn apic_timer_ms_to_ticks(ms: u64) -> u32 {
        let ticks_per_ms = unsafe { core::ptr::read(APIC_TIMER_TICKS_PER_MS.get()) };
        (ms * ticks_per_ms) as u32
    }

    let addr = get_lapic_reg(local_apic_addr, 0x320);
    let init = get_lapic_reg(local_apic_addr, 0x380);
    let divide = get_lapic_reg(local_apic_addr, 0x3E0).cast::<u8>();
    let current_counter = get_lapic_reg(local_apic_addr, 0x390);

    // calibrate the timer
    unsafe {
        serial!("calibrating the apic timer\n");
        let timer = LVTEntry::new(0x81, LVTEntryFlags::empty());

        core::ptr::write_volatile(addr, timer.encode_u32());
        core::ptr::write_volatile(divide, 0x3);
        pit::prepare_sleep(100);

        core::ptr::write_volatile(init, u32::MAX);

        asm!("sti");
        let diff_tick = pit::calibrate_sleep(
            lapic_id,
            || (),
            |()| u32::MAX - current_counter.read_volatile(),
        );
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

pub fn calibrate_tsc(lapic_id: u8, ticks_per_ms: &mut u64) {
    static _CALIBRATE_LOCK: SpinLock<()> = SpinLock::new(());
    let _guard = _CALIBRATE_LOCK.lock();
    serial!("calbrating tsc\n");
    unsafe {
        pit::prepare_sleep(100);

        asm!("sti");
        let diff_tick = pit::calibrate_sleep(
            lapic_id,
            || core::arch::x86_64::_rdtsc(),
            |x| core::arch::x86_64::_rdtsc() - x,
        );
        asm!("cli");

        *ticks_per_ms = diff_tick / 100;
        info!("calibrated TSC with {} ticks in 100ms", *ticks_per_ms);
    }
}

const PIC1_DATA: u16 = 0x0021;
const PIC2_DATA: u16 = 0x00A1;

fn disable_pic() {
    outb(PIC1_DATA, 0xff);
    outb(PIC2_DATA, 0xff);
}
fn enable_apic() {
    let lapic_base = *LAPIC_PHYS_ADDR;
    const IA32_APIC_BASE_MSR: u32 = 0x1B;
    const IA32_APIC_BASE_MSR_ENABLE: u32 = 0x800;
    unsafe {
        wrmsr(
            IA32_APIC_BASE_MSR,
            lapic_base.into_raw() as u64 | IA32_APIC_BASE_MSR_ENABLE as u64,
        );
    }
}
/// Genericly enables APIC interrupts and the APIC timer for the current CPU, fills `tsc_ticks_per_ms_output` with the amount of ticks per a ms in the TSC
pub fn enable_apic_interrupts_generic(tsc_ticks_per_ms_output: &mut u64) {
    disable_pic();
    enable_apic();
    let lapic_addr = get_lapic_addr();
    let sivr = get_lapic_reg(lapic_addr, 0xF0);

    unsafe {
        core::ptr::write_volatile(sivr, 0x1ff);

        let ioapic_addr = get_io_apic_addr();

        let lapic_id = get_lapic_id(lapic_addr);
        let ioapic_id = get_ioapic_id(ioapic_addr);

        info!(
            "enabled APIC, lapic_id is {lapic_id}, ioapic_id is {ioapic_id}, IO APIC is at {ioapic_addr:#x}, local APIC is at {lapic_addr:#x}"
        );
        static _ENABLE_LOCK: SpinLock<()> = SpinLock::new(());
        let _guard = _ENABLE_LOCK.lock();

        configure_error(lapic_addr);
        calibrate_tsc(lapic_id, tsc_ticks_per_ms_output);
        enable_apic_timer(lapic_addr, lapic_id);
    }
}
