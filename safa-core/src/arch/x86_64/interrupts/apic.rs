use super::pit;
use crate::{
    PhysAddr, VirtAddr,
    arch::{
        registers::CPUID,
        x86_64::{
            acpi,
            interrupts::handlers::{APIC_ERROR_HANDLER_ID, MOUSE_HANDLER_ID, NMI_REASON},
            io::outb,
            registers::{rdmsr, wrmsr},
        },
    },
    info,
    memory::vmm::{VMMAllocError, VMMMFlags, VirtualMemoryManager},
    serial,
    utils::locks::{LazyLock, SpinLock},
};
use bitfield_struct::bitfield;
use bitflags::bitflags;
use core::{cell::UnsafeCell, num::NonZero};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum APICDeliveryMode {
    /// Delivers the interrupt specified in the vector field to the target processor or processors.
    Fixed = 0,
    /// Same as fixed mode, except that the interrupt is delivered to the processor executing at the lowest priority among the set of processors specified in the destination field. The ability for a processor to send a lowest priority IPI is model specific and should be avoided by BIOS and operating system software.
    LowestPiriority = 1,
    /// Delivers an SMI interrupt to the target processor or processors. The vector field must be programmed to 00H for future compatibility.
    SMI = 0b010,
    Reserved = 0b011,
    /// Delivers an NMI interrupt to the target processor or processors. The vector information is ignored.
    NMI = 0b100,
    /// Delivers an INIT request to the target processor or processors, which causes them to perform an INIT.
    INIT = 0b101,
    /// Sends a special start-up IPI (called a SIPI) to the target processor or processors.
    ///
    /// The vector typically points to a start-up routine that is part of the BIOS boot-strap code
    /// (see Section 8.4, Multiple-Processor (MP) Initialization).
    ///
    /// IPIs sent with this delivery mode are not automatically retried if the source APIC is unable to deliver it.
    ///
    /// It is up to the software to deter- mine if the SIPI was not successfully delivered and to reissue the SIPI if necessary.
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
    /// No short hand. the destination is controlled by the other interrupt register.
    NoShortHand = 0,
    /// Send to only Self
    SelfOnly = 1,
    /// Send to all CPUs
    All = 2,
    /// Send to all CPUs excluding Self
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
    /// Clear for INIT level de-assert, otherwise set.
    /// Level
    ///
    /// 0 == De-assert
    ///
    /// 1 == Assert
    no_init_level_deassert: bool,
    /// Set for INIT level de-assert, otherwise clear
    init_level_deassert: bool,
    #[bits(2)]
    __: (),
    #[bits(2)]
    destination_shorthand: APICDestShorthand,
    #[bits(36)]
    __: (),
    destination_field: u8,
}

/// The APIC driver
pub struct Apic {
    lapic_phys_addr: PhysAddr,
    lapic_virt_addr: UnsafeCell<VirtAddr>,
    ioapic_phys_addr: PhysAddr,
    ioapic_virt_addr: UnsafeCell<VirtAddr>,
}

impl Apic {
    /// Gets the APIC if it is there
    pub fn get() -> Option<Self> {
        let lapic_phys = rdmsr(0x1B) & 0xFFFFF000;
        let lapic_phys_addr = PhysAddr::from(lapic_phys);

        let ioapic_phys_addr = unsafe {
            let madt = (*acpi::MADT_DESC)?;
            let record = madt.get_record_of_type(1).unwrap() as *const MADTIOApic;

            let addr = PhysAddr::from((*record).ioapic_address as usize);
            addr
        };
        Some(Self {
            ioapic_phys_addr,
            ioapic_virt_addr: UnsafeCell::new(VirtAddr::null()),
            lapic_virt_addr: UnsafeCell::new(VirtAddr::null()),
            lapic_phys_addr,
        })
    }
    /// Maps the IOAPIC and the Local APIC to the `dest` VMM
    ///
    /// # Safety:
    /// Must be called only once per APIC Driver
    pub fn map(&self, dest: &mut VirtualMemoryManager) -> Result<(), VMMAllocError> {
        let flags = VMMMFlags::WRITEABLE | VMMMFlags::UNCACHABLE;

        let lapic_addr =
            dest.map_direct_phys(&"LOCAL APIC", None, self.lapic_phys_addr, 1, flags)?;
        let io_apic_addr =
            dest.map_direct_phys(&"IO APIC", None, self.ioapic_phys_addr, 1, flags)?;

        unsafe {
            *self.lapic_virt_addr.get() = lapic_addr;
            *self.ioapic_virt_addr.get() = io_apic_addr;
        }
        Ok(())
    }

    #[inline(always)]
    const fn get_lapic_reg_addr(&self, lapic_reg: u16) -> VirtAddr {
        unsafe { *self.lapic_virt_addr.get() + lapic_reg as usize }
    }

    #[inline(always)]
    fn get_lapic_reg(&self, lapic_off: u16) -> *mut u32 {
        self.get_lapic_reg_addr(lapic_off).into_ptr::<u32>()
    }

    #[inline(always)]
    fn read_lapic_reg(&self, lapic_off: u16) -> u32 {
        unsafe {
            // performs a dword read as expected from the local APIC
            self.get_lapic_reg(lapic_off).read_volatile()
        }
    }

    /// CPU local APIC ID.
    #[inline]
    pub fn lapic_id(&self) -> u8 {
        (self.read_lapic_reg(0x20) >> 24) as u8
    }

    pub unsafe fn write_ioapic_val_to_reg(&self, reg: u8, val: u32) {
        unsafe {
            let ioregsel_addr = (*self.ioapic_virt_addr.get()).into_ptr::<u32>();
            let iowin_addr = (*self.ioapic_virt_addr.get() + 0x10).into_ptr::<u32>();

            core::ptr::write_volatile(ioregsel_addr, reg as u32);
            core::ptr::write_volatile(iowin_addr, val);
        }
    }

    pub fn read_ioapic_reg(&self, reg: u8) -> u32 {
        unsafe {
            let ioregsel_addr = (*self.ioapic_virt_addr.get()).into_ptr::<u32>();
            let iowin_addr = (*self.ioapic_virt_addr.get() + 0x10).into_ptr::<u32>();

            core::ptr::write_volatile(ioregsel_addr, reg as u32);
            core::ptr::read_volatile(iowin_addr)
        }
    }

    #[inline]
    pub fn ioapic_id(&self) -> u8 {
        (self.read_ioapic_reg(0) >> 24) as u8
    }

    pub fn write_ic_reg(&self, value: APICICReg) {
        let low = self.get_lapic_reg(0x300);
        let high = self.get_lapic_reg(0x310);

        let value_bits = value.into_bits();
        let (value_low, value_high) = (value_bits as u32, (value_bits >> 32) as u32);
        unsafe {
            high.write_volatile(value_high);
            low.write_volatile(value_low);
        }
    }

    pub fn read_ic_reg(&self) -> APICICReg {
        let low = self.get_lapic_reg(0x300);
        let high = self.get_lapic_reg(0x310);
        unsafe {
            let low_bits = low.read_volatile();
            let high_bits = high.read_volatile();
            let bits: u64 = low_bits as u64 | (high_bits as u64) << 32;
            APICICReg::from_bits(bits)
        }
    }

    #[inline]
    pub fn send_eoi(&self) {
        unsafe {
            let eoi_reg = self.get_lapic_reg(0xB0);
            eoi_reg.write_volatile(0);
        }
    }

    #[inline]
    /// Sends an NMI to all processors
    pub fn send_nmi_all(&self, reason: usize) {
        static _NMI_SEND: SpinLock<()> = SpinLock::new(());
        let _guard = _NMI_SEND.lock();
        NMI_REASON.store(reason, core::sync::atomic::Ordering::Relaxed);

        self.write_ic_reg(
            APICICReg::new()
                .with_delivery_mode(APICDeliveryMode::NMI)
                .with_destination_shorthand(APICDestShorthand::ExcludingSelf),
        );

        while self.read_ic_reg().delivery_send_pending() {
            core::hint::spin_loop();
        }
    }

    #[inline]
    /// Send an IPI to a target CPU
    pub fn send_ipi_to(&self, vector: u8, target: CPUID) {
        self.write_ic_reg(
            APICICReg::new()
                .with_destination_shorthand(APICDestShorthand::NoShortHand)
                .with_vector(vector)
                .with_destination_field(target.lapic_id()),
        );

        while self.read_ic_reg().delivery_send_pending() {
            core::hint::spin_loop()
        }
    }

    #[inline]
    pub unsafe fn write_ioapic_irq(&self, n: u8, table: IOREDTBL) {
        unsafe {
            let offset1 = 0x10 + (n * 2);
            let offset2 = offset1 + 1;

            let (lower, higher) = table.into_regs();
            self.write_ioapic_val_to_reg(offset1, lower);
            self.write_ioapic_val_to_reg(offset2, higher);
        }
    }

    pub fn enable_apic_keyboard(&self) {
        unsafe {
            let lapic_id = self.lapic_id();

            let keyboard = IOREDTBL::new().with_vector(0x21).with_destination(lapic_id);
            self.write_ioapic_irq(1, keyboard);

            info!("enabled APIC Keyboard for lapic {lapic_id}.");
        }
    }

    pub fn enable_apic_mouse(&self) {
        unsafe {
            let lapic_id = self.lapic_id();

            let mouse = IOREDTBL::new()
                .with_vector(MOUSE_HANDLER_ID)
                .with_destination(lapic_id);
            self.write_ioapic_irq(12, mouse);

            info!("enabled APIC mouse for lapic {lapic_id}.");
        }
    }

    fn configure_error(&self) {
        let addr = self.get_lapic_reg(0x370);
        let entry = LVTEntry::new(APIC_ERROR_HANDLER_ID, LVTEntryFlags::empty());
        unsafe {
            addr.write_volatile(entry.encode_u32());
        }
    }

    pub fn enable_apic_timer(&self, tsc_frequency: NonZero<u64>) {
        let lapic_id = self.lapic_id();

        info!("enabling apic timer for lapic: {lapic_id}...");

        let ticks_per_ms;

        let addr = self.get_lapic_reg(0x320);
        let init = self.get_lapic_reg(0x380);
        let divide = self.get_lapic_reg(0x3E0).cast::<u8>();
        let current_counter = self.get_lapic_reg(0x390);

        // calibrate the timer
        unsafe {
            const SLEEP_MS: u32 = 10;
            serial!("calibrating the apic timer\n");

            let timer = LVTEntry::new(0x81, LVTEntryFlags::empty());
            core::ptr::write_volatile(addr, timer.encode_u32());
            core::ptr::write_volatile(divide, 0x3);

            core::ptr::write_volatile(init, u32::MAX);

            let read_apic_count = || u32::MAX - current_counter.read_volatile();
            let read_tsc_ticks = || crate::arch::utils::cpu_cycles();

            // Calibration
            let beginning_apic = read_apic_count();
            let beginning_tsc = read_tsc_ticks();

            let end_tsc = beginning_tsc + (tsc_frequency.get() * (SLEEP_MS * 1000) as u64);
            // Loop until TSC reaches the count and [SLEEP_MS] has passed.
            while read_tsc_ticks() < end_tsc {}
            let end_apic = read_apic_count();

            ticks_per_ms = (end_apic - beginning_apic) as u64 / SLEEP_MS as u64;
            info!(
                "APIC Timer calibrated with {} ticks in 100ms",
                ticks_per_ms * 100
            );
        }

        // enable the timer
        unsafe {
            let timer = LVTEntry::new(0x20, LVTEntryFlags::TIMER_PERIODIC);
            core::ptr::write_volatile(addr, timer.encode_u32());
            core::ptr::write_volatile(divide, 0x3);

            core::ptr::write_volatile(init, 5 * ticks_per_ms as u32);
        }
    }

    fn enable_apic(&self) {
        const PIC1_DATA: u16 = 0x0021;
        const PIC2_DATA: u16 = 0x00A1;

        // Disable PIC
        unsafe {
            outb(PIC1_DATA, 0xff);
            outb(PIC2_DATA, 0xff);
        }

        let lapic_base = self.lapic_phys_addr;
        const IA32_APIC_BASE_MSR: u32 = 0x1B;
        const IA32_APIC_BASE_MSR_ENABLE: u32 = 0x800;
        unsafe {
            wrmsr(
                IA32_APIC_BASE_MSR,
                lapic_base.into_raw() as u64 | IA32_APIC_BASE_MSR_ENABLE as u64,
            );
        }
    }

    /// Calibrates the TSC and setups the APIC timer returning the number of ticks per microsecond (frequency in MHz) after calibration of the TSC.
    fn setup_timer(&self) -> NonZero<u64> {
        let freq = calibrate_tsc();
        self.enable_apic_timer(freq);
        freq
    }

    /// Setups the APIC and related devices for the current CPU
    fn setup_local(&self) {
        self.enable_apic();
        let sivr = self.get_lapic_reg(0xF0);

        unsafe {
            core::ptr::write_volatile(sivr, 0x1ff);

            let lapic_id = self.lapic_id();
            let ioapic_id = self.ioapic_id();

            info!(
                "enabled APIC, lapic_id is {lapic_id}, ioapic_id is {ioapic_id}, IO APIC is at {:#x}, local APIC is at {:#x}",
                *self.ioapic_virt_addr.get(),
                *self.lapic_virt_addr.get(),
            );

            self.configure_error();
        }
    }

    /// Returns the base addr of the lapic
    pub const fn lapic_base(&self) -> PhysAddr {
        self.lapic_phys_addr
    }
}

unsafe impl Sync for Apic {}

pub static APIC: LazyLock<Apic> = LazyLock::new(|| Apic::get().expect("Apic not supported"));

/// Sends an NMI to all processors
pub fn send_nmi_all(reason: usize) {
    // If not then the apic isn't initialized and so is other processors
    if let Some(apic) = APIC.get() {
        apic.send_nmi_all(reason)
    }
}

#[inline]
/// Send an IPI to a target CPU
pub fn send_ipi_to(vector: u8, target: CPUID) {
    // If not then the apic isn't initialized and so is other processors
    if let Some(apic) = APIC.get() {
        apic.send_ipi_to(vector, target)
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
    APIC.send_eoi()
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

    timer_periodic: bool,
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

/// Setups APIC interrupts for the PS/2 keyboard
pub fn enable_apic_keyboard() {
    APIC.enable_apic_keyboard()
}

/// Setups APIC interrupts for the PS/2 mouse
pub fn enable_apic_mouse() {
    APIC.enable_apic_mouse()
}

/// Returns the frequency in MHz of the TSC once calibrated
pub fn calibrate_tsc() -> NonZero<u64> {
    static _CALIBRATE_LOCK: SpinLock<()> = SpinLock::new(());
    let _guard = _CALIBRATE_LOCK.lock();
    serial!("calbrating tsc\n");
    unsafe {
        let freq = pit::calibrate_tsc();
        info!("calibrated TSC with {} ticks in 1us", freq);
        freq
    }
}

/// Genericly enables APIC interrupts for the current CPU
pub fn enable_apic_interrupts_generic() {
    APIC.setup_local()
}

/// Calibrates the TSC and setups the APIC timer returning (frequency) after calibration of the TSC.
pub fn setup_timer() -> NonZero<u64> {
    APIC.setup_timer()
}

/// Maps the IOAPIC and the Local APIC to the `dest` VMM
pub fn map_apic(dest: &mut VirtualMemoryManager) -> Result<(), VMMAllocError> {
    APIC.map(dest)
}
