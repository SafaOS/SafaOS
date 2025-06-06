use bitfield_struct::bitfield;
use lazy_static::lazy_static;

use crate::{
    arch::{aarch64::cpu, paging::current_higher_root_table},
    debug, info,
    memory::paging::{EntryFlags, MapToError, PAGE_SIZE},
    VirtAddr,
};

use super::paging::PageTable;
pub mod cpu_if;

lazy_static! {
    static ref GICC: Option<(VirtAddr, usize)> =
        cpu::GICV3.0.map(|(base, size)| (base.into_virt(), size));
    static ref GICD: (VirtAddr, usize) = {
        let (base, size) = cpu::GICV3.1;
        (base.into_virt(), size)
    };
    static ref GICR: (VirtAddr, usize) = {
        let (base, size) = cpu::GICV3.2;
        (base.into_virt(), size)
    };
    static ref GICD_BASE: VirtAddr = GICD.0;
    static ref GICD_SIZE: usize = GICD.1;
    static ref GICR_BASE: VirtAddr = GICR.0;
    static ref GICR_SIZE: usize = GICR.1;
    static ref SGI_BASE: VirtAddr = *GICR_BASE + (/* 64 KiB */ 64 * 1024);
}

#[bitfield(u64)]
struct GICRTyper {
    plpis: bool,
    vlpis: bool,
    dirty: bool,
    direct_lpi: bool,
    last: bool,
    dpgs: bool,
    mpam: bool,
    rvpeid: bool,
    processor_num: u16,
    #[bits(2)]
    common_lpi_af: u8,
    vsgi: bool,
    #[bits(5)]
    ppi_num: u8,
    af_value: u32,
}

impl GICRTyper {
    fn get() -> Self {
        unsafe { Self::from_bits(core::ptr::read((*GICR_BASE + 0x8).into_ptr::<u64>())) }
    }
}

#[bitfield(u32)]
struct GICDTyper {
    #[bits(5)]
    /// For the INTID range 32 to 1019, indicates the maximum SPI supported.
    ///
    /// If the value of this field is N, the maximum SPI INTID is 32(N+1) minus 1. For example, 00011
    /// specifies that the maximum SPI INTID is 127.
    ///
    /// Regardless of the range of INTIDs defined by this field, interrupt IDs 1020-1023 are reserved for
    /// special purposes.
    ///
    /// A value of 0 indicates no SPIs are supported.
    ///
    /// RO
    it_lines_num: u8,
    #[bits(3)]
    /// Reports the number of PEs that can be used when affinity routing is not enabled, minus 1.
    ///
    /// These PEs must be numbered contiguously from zero, but the relationship between this number and
    /// the affinity hierarchy from MPIDR is IMPLEMENTATION DEFINED.
    /// If the implementation does not
    /// support ARE being zero, this field is 000.
    ///
    /// RO
    cpu_num: u8,
    /// Extended SPI.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 Extended SPI range not implemented.
    ///
    /// 0b1 Extended SPI range implemented.
    ///
    /// Access to this field is RO.
    espi: bool,
    /// Non-maskable Interrupts.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 Non-maskable interrupt property not supported.
    ///
    /// 0b1 Non-maskable interrupt property is supported.
    ///
    /// Access to this field is RO.
    nmi: bool,
    /// Indicates whether the GIC implementation supports two Security states:
    ///
    /// When GICD_CTLR.DS == 1, this field is RAZ.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The GIC implementation supports only a single Security state.
    ///
    /// 0b1 The GIC implementation supports two Security states.
    ///
    /// Access to this field is RO.
    security_ext: bool,
    #[bits(5)]
    /// Number of supported LPIs.
    ///
    /// • 0b00000 Number of LPIs as indicated by GICD_TYPER.IDbits.
    ///
    /// • All other values Number of LPIs supported is 2(num_LPIs+1) .
    ///
    /// — Available LPI INTIDs are 8192..(8192 + 2(num_LPIs+1) - 1).
    ///
    /// — This field cannot indicate a maximum LPI INTID greater than that indicated by
    /// GICD_TYPER.IDbits.
    ///
    /// When the supported INTID width is less than 14 bits, this field is RES0 and no LPIs are supported.
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    num_lpis: u8,
    /// Indicates whether the implementation supports message-based interrupts by writing to Distributor
    /// registers.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The implementation does not support message-based interrupts by writing to
    /// Distributor registers.
    ///
    /// The GICD_CLRSPI_NSR, GICD_SETSPI_NSR, GICD_CLRSPI_SR, and
    /// GICD_SETSPI_SR registers are reserved.
    ///
    /// 0b1 The implementation supports message-based interrupts by writing to the
    /// GICD_CLRSPI_NSR, GICD_SETSPI_NSR, GICD_CLRSPI_SR, or
    /// GICD_SETSPI_SR registers.
    ///
    /// Access to this field is RO.
    mbis: bool,
    /// Indicates whether the implementation supports LPIs.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The implementation does not support LPIs.
    ///
    /// 0b1 The implementation supports LPIs.
    ///
    /// Access to this field is RO.
    lpis: bool,
    /// When FEAT_GICv4 is implemented:
    ///
    /// Indicates whether the implementation supports Direct Virtual LPI injection.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The implementation does not support Direct Virtual LPI injection.
    ///
    /// 0b1 The implementation supports Direct Virtual LPI injection.
    ///
    /// Access to this field is RO.
    dvis: bool,
    #[bits(5)]
    /// The number of interrupt identifier bits supported, minus one.
    ///
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    id_bits: u8,
    /// Affinity 3 valid. Indicates whether the Distributor supports nonzero values of Affinity level 3.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The Distributor only supports zero values of Affinity level 3.
    ///
    /// 0b1 The Distributor supports nonzero values of Affinity level 3.
    ///
    /// Access to this field is RO.
    a3v: bool,
    /// Indicates whether 1 of N SPI interrupts are supported.
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 1 of N SPI interrupts are supported.
    ///
    /// 0b1 1 of N SPI interrupts are not supported.
    ///
    /// Access to this field is RO.
    no1n: bool,
    /// Range Selector Support.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The IRI supports targeted SGIs with affinity level 0 values of 0 - 15.
    ///
    /// 0b1 The IRI supports targeted SGIs with affinity level 0 values of 0 - 255.
    ///
    /// Access to this field is RO.
    rss: bool,
    #[bits(5)]
    espi_range: u8,
}

impl GICDTyper {
    fn get() -> Self {
        unsafe { Self::from_bits(*(*GICD_BASE + 0x4).into_ptr::<u32>()) }
    }
}

#[bitfield(u32)]
struct GICRWaker {
    /// IMPLEMENTITION DEFINED
    #[bits(1)]
    __: (),

    processor_sleep: bool,
    children_asleep: bool,

    #[bits(28)]
    __: (),

    /// IMPLEMENTITION DEFINED
    #[bits(1)]
    __: (),
}

impl GICRWaker {
    pub fn get_ptr() -> *mut Self {
        (*GICR_BASE + 0x14).into_ptr::<_>()
    }
    /// Performs a volitate write to the GICR_WAKER register
    pub fn write(src: Self) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(), src);
        }
    }
    /// Performs a volitate read to retrieve self
    pub fn read_vol() -> Self {
        unsafe { core::ptr::read_volatile(Self::get_ptr()) }
    }
}

// Assumes system is executing in secure with 2 security states
#[bitfield(u32)]
struct GICDCtlr {
    /// Enable Group 0 interrupts
    enable_grp0: bool,
    /// Enable Non-secure Group 1 interrupts
    enable_grp1_non_sec: bool,
    /// Enable Secure Group 1 interrupts
    enable_grp1_sec: bool,
    #[bits(1)]
    __: (),
    /// Affinity Routing Enable, Secure state
    are_sec: bool,
    /// Affinity Routing Enable, Non-secure state.
    are_non_sec: bool,
    /// Disable Security.
    disable_sec: bool,
    /// Enable 1 of N Wakeup Functionality.
    /// It is IMPLEMENTATION DEFINED whether this bit is programmable, or RAZ/WI.
    ///
    /// If it is implemented, then it has the following behavior:
    ///
    /// 0b0 A PE that is asleep cannot be picked for 1 of N interrupts.
    ///
    /// 0b1 A PE that is asleep can be picked for 1 of N interrupts as determined by
    /// IMPLEMENTATION DEFINED controls.
    e1nwf: bool,
    #[bits(23)]
    __: (),
    /// Register Write Pending. Read only. Indicates whether a register write is in progress or not:
    ///
    /// 0b0 No register write in progress. The effects of previous register writes to the affected
    /// register fields are visible to all logical components of the GIC architecture, including
    /// the CPU interfaces.
    ///
    /// 0b1 Register write in progress. The effects of previous register writes to the affected register
    /// fields are not guaranteed to be visible to all logical components of the GIC architecture,
    /// including the CPU interfaces, as the effects of the changes are still being propagated.
    reg_write_pending: bool,
}

impl GICDCtlr {
    #[inline(always)]
    pub fn get_ptr() -> *mut Self {
        GICD_BASE.into_ptr::<_>()
    }
}

unsafe fn map_gic(dest: &mut PageTable) -> Result<(), MapToError> {
    let flags = EntryFlags::WRITE;
    if let Some((gicc_base, size)) = *GICC {
        dest.map_contiguous_pages(
            gicc_base,
            gicc_base.into_phys(),
            size.div_ceil(PAGE_SIZE),
            flags,
        )?;
    }
    dest.map_contiguous_pages(
        *GICD_BASE,
        (*GICD_BASE).into_phys(),
        (*GICD_SIZE).div_ceil(PAGE_SIZE),
        flags,
    )?;
    dest.map_contiguous_pages(
        *GICR_BASE,
        (*GICR_BASE).into_phys(),
        (*GICR_SIZE).div_ceil(PAGE_SIZE),
        flags,
    )?;
    Ok(())
}

pub fn init_gic() {
    unsafe {
        map_gic(&mut *current_higher_root_table()).expect("failed to map gic");
    }
    info!(
        "initializing GIC GICD: {:?}, GICR: {:?}",
        *GICD_BASE, *GICR_BASE
    );

    let gicd_ctlr = GICDCtlr::get_ptr();
    unsafe {
        core::ptr::write_volatile(
            gicd_ctlr,
            GICDCtlr::default()
                .with_are_sec(true)
                .with_are_non_sec(true)
                .with_enable_grp0(true)
                .with_enable_grp1_sec(true)
                .with_enable_grp1_non_sec(true),
        );

        let gicd_typer = GICDTyper::get();
        info!(
            "configured the GICD, max SPI intID: {}, LPIs support: {}, nmi: {}, 2 security states: {}, SPI MSI support: {}",
            ((gicd_typer.it_lines_num() as u16 + 1) * 32) - 1,
            gicd_typer.lpis(),
            gicd_typer.nmi(),
            gicd_typer.security_ext(),
            gicd_typer.mbis(),
        );

        GICRWaker::write(GICRWaker::new().with_processor_sleep(false));
        assert!(!GICRWaker::read_vol().processor_sleep());
        // Polls until it wakes uo
        while GICRWaker::read_vol().children_asleep() {
            core::hint::spin_loop();
        }

        let gicr_typer = GICRTyper::get();
        info!(
            "woke up the GICR, processor num: {}, is the last GICR: {}, supports direct lpis: {}",
            gicr_typer.processor_num(),
            gicr_typer.last(),
            gicr_typer.direct_lpi()
        );

        cpu_if::init();
    }
}

#[derive(Debug, Clone, Copy)]
pub enum IntKind {
    PPI,
    SGI,
    SPI,
    LPI,
}

impl IntKind {
    // TODO: cleanup code and use this function more
    pub const fn from_int_id(int_id: u32) -> Self {
        match int_id {
            0..=15 => Self::SGI,
            16..=31 => Self::PPI,
            32..=1019 => Self::SPI,
            8192.. => Self::LPI,
            // Special and reserved
            1020..=1023 | 1024..=8191 => unreachable!(),
        }
    }

    const fn choose_reg<T>(self, gicd_reg: *mut T, gicr_reg: *mut T) -> *mut T {
        match self {
            Self::SGI | Self::PPI => gicr_reg,
            Self::SPI => gicd_reg,
            Self::LPI => todo!(),
        }
    }
}

#[inline(always)]
fn gicd_isenabler() -> *mut u32 {
    (*GICD_BASE + 0x100).into_ptr::<u32>()
}

#[inline(always)]
fn gicr_isenabler() -> *mut u32 {
    (*SGI_BASE + 0x100).into_ptr::<u32>()
}

/// Enables GIC interrupt
fn enable(interrupt: u32, int_kind: IntKind) {
    let value = interrupt % 32;
    let index = interrupt / 32;

    unsafe {
        let reg = int_kind.choose_reg(gicd_isenabler(), gicr_isenabler());
        core::ptr::write_volatile(reg.add(index as usize), 1 << value);
    }
}

#[inline(always)]
fn gicd_icpendr0() -> *mut u32 {
    (*GICD_BASE + 0x0280).into_ptr::<u32>()
}

#[inline(always)]
fn gicr_icpendr0() -> *mut u32 {
    (*SGI_BASE + 0x0280).into_ptr::<u32>()
}

#[inline(always)]
/// Clears pending interrupt
pub fn clear_pending(interrupt: u32, kind: IntKind) {
    let value = interrupt % 32;
    let index = interrupt / 32;
    unsafe {
        let reg = kind.choose_reg(gicd_icpendr0(), gicr_icpendr0());
        core::ptr::write_volatile(reg.add(index as usize), 1 << value);
    }
}

#[inline(always)]
fn gicr_igroup0() -> *mut u32 {
    (*SGI_BASE + 0x0080).into_ptr::<u32>()
}

#[inline(always)]
fn gicd_igroup0() -> *mut u32 {
    (*GICD_BASE + 0x0080).into_ptr::<u32>()
}

/// The interrupt group
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntGroup {
    /// Group 0
    NonSecure = 0,
    /// Group 1
    Secure = 1,
}

#[inline(always)]
pub fn set_group(int_id: u32, kind: IntKind, group: IntGroup) {
    let value = int_id % 32;
    let index = int_id / 32;

    unsafe {
        let reg = kind.choose_reg(gicd_igroup0(), gicr_igroup0());
        let reg = reg.add(index as usize);

        let shift = 1 << value;
        core::ptr::write_volatile(
            reg,
            if group == IntGroup::Secure {
                *reg | shift
            } else {
                *reg & !shift
            },
        );
    }
}

fn gicd_ispendr0() -> *mut u32 {
    (*GICD_BASE + 0x200).into_ptr::<u32>()
}

fn gicr_ispendr0() -> *mut u32 {
    (*SGI_BASE + 0x200).into_ptr::<u32>()
}

fn set_pending(int_id: u32, kind: IntKind) {
    let value = int_id % 32;
    let index = int_id / 32;
    let reg = kind.choose_reg(gicd_ispendr0(), gicr_ispendr0());
    unsafe {
        let reg = reg.add(index as usize);
        core::ptr::write_volatile(reg, 1 << value);
    }
}

/// A Generic Wrapper around a GIC interrupt
pub struct IntID {
    id: u32,
    kind: IntKind,
}

impl IntID {
    pub const fn from_int_id(int_id: u32) -> Self {
        Self {
            id: int_id,
            kind: IntKind::from_int_id(int_id),
        }
    }
    /// Enables the interrupt
    pub fn enable(&self) -> &Self {
        enable(self.id, self.kind);
        debug!(IntID, "enabled interrupt with ID `{}`", self.id);
        self
    }
    /// Marks the interrupt as not pending
    pub fn clear_pending(&self) -> &Self {
        clear_pending(self.id, self.kind);
        self
    }
    /// Makes the interrupt pending
    pub fn set_pending(&self) -> &Self {
        set_pending(self.id, self.kind);
        self
    }
    /// Sets the group of the interrupt to `group`
    pub fn set_group(&self, group: IntGroup) -> &Self {
        set_group(self.id, self.kind, group);
        debug!(
            IntID,
            "set group of interrupt with ID `{}` to {:?}", self.id, group
        );
        self
    }

    pub fn id(&self) -> u32 {
        self.id
    }
}
