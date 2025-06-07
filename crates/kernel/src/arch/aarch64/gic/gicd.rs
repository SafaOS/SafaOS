use crate::info;

use super::GICD_BASE;
use bitfield_struct::bitfield;

/// Initializes the GICD
pub fn init() {
    unsafe {
        let gicd_ctlr = GICDCtlr::get_ptr();

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
           gicd_typer.has_lpis(),
           gicd_typer.nmi(),
           gicd_typer.security_ext(),
           gicd_typer.mbis(),
       );
        assert!(
            gicd_typer.has_lpis(),
            "no LPIs no MSIs no XHCI no USB no OS until AML for now"
        );
    }
}

#[bitfield(u32)]
pub struct GICDTyper {
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
    has_lpis: bool,
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

// Assumes system is executing in secure with 2 security states
#[bitfield(u32)]
pub struct GICDCtlr {
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

/// Pointer to the GICD_ISENABLER<0> Register
#[inline(always)]
pub fn isenabler() -> *mut u32 {
    (*GICD_BASE + 0x100).into_ptr::<u32>()
}

/// Pointer to the GICD_ICPENDR<0> Register
#[inline(always)]
pub fn icpendr0() -> *mut u32 {
    (*GICD_BASE + 0x0280).into_ptr::<u32>()
}

/// Pointer to the GICD_IGROUP<0> Register
#[inline(always)]
pub fn igroup0() -> *mut u32 {
    (*GICD_BASE + 0x0080).into_ptr::<u32>()
}

/// Pointer to the GICD_ISPENDR0<0> Register
#[inline(always)]
pub fn ispendr0() -> *mut u32 {
    (*GICD_BASE + 0x200).into_ptr::<u32>()
}
