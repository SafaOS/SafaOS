use bitfield_struct::bitfield;
use lazy_static::lazy_static;

use crate::{
    arch::aarch64::gic::{
        its::commands::{ITSCommand, GITS_COMMAND_QUEUE},
        GICITS_BASE, GICITS_TRANSLATION_BASE, GICR_BASE,
    },
    debug,
    memory::{
        align_up,
        frame_allocator::{self, SIZE_1K, SIZE_64K_PAGES},
        paging::PAGE_SIZE,
    },
    VirtAddr,
};

pub mod commands;

#[bitfield(u64)]
pub struct GITSTyper {
    /// Indicates whether the ITS supports physical LPIs:
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The ITS does not support physical LPIs.
    ///
    /// 0b1 The ITS supports physical LPIs.
    ///
    /// This field is RES1, indicating that the ITS supports physical LPIs.
    ///
    /// Access to this field is RO.
    physical: bool,
    /// When FEAT_GICv4 is implemented:
    ///
    /// Indicates whether the ITS supports virtual LPIs and direct injection of virtual LPIs:
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The ITS does not support virtual LPIs or direct injection of virtual LPIs.
    ///
    /// 0b1 The ITS supports virtual LPIs and direct injection of virtual LPIs.
    ///
    /// Access to this field is RO.
    virtual_lpis: bool,
    /// Cumulative Collection Tables.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The total number of supported collections is determined by the number of collections
    /// held in memory only.
    ///
    /// 0b1 The total number of supported collections is determined by number of collections that
    /// are held in memory and the number indicated by GITS_TYPER.HCC.
    ///
    /// If GITS_TYPER.HCC == 0, or if memory backed collections are not supported (all
    /// GITS_BASER<n>.Type != 100), this bit is RES0.
    cct: bool,
    #[bits(1)]
    __: (),
    /// Read-only. Indicates the number of bytes per translation table entry, minus one.
    ///
    /// For more information about the ITS command 'MAPD', see MAPD on page 5-112.
    ///
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    #[bits(4)]
    itt_entry_size: u8,
    /// The number of EventID bits implemented, minus one.
    ///
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    #[bits(5)]
    event_id_bits: u8,
    /// The number of DeviceID bits implemented, minus one.
    ///
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    #[bits(5)]
    device_id_bits: u8,
    /// SEI support. Indicates whether the virtual CPU interface supports generation of SEIs:
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The ITS does not support local generation of SEIs.
    ///
    /// 0b1 The ITS supports local generation of SEIs.
    ///
    /// Access to this field is RO.
    seis: bool,
    /// Physical Target Addresses. Indicates the format of the target address:
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 The target address corresponds to the PE number specified by
    /// GICR_TYPER.Processor_Number.
    ///
    /// 0b1 The target address corresponds to the base physical address of the required
    /// Redistributor.
    ///
    /// For more information, see RDbase.
    ///
    /// Access to this field is RO.
    pta_base_addr: bool,
    #[bits(4)]
    __: (),

    /// Hardware Collection Count. The number of interrupt collections supported by the ITS without
    /// provisioning of external memory
    ///
    /// Collections held in hardware are unmapped at reset.
    ///
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    hcc: u8,
    /// Number of Collection ID bits.
    ///
    /// • The number of bits of Collection ID minus one.
    ///
    /// • When GITS_TYPER.CIL == 0, this field is RES0.
    ///
    /// This field has an IMPLEMENTATION DEFINED value.
    ///
    /// Access to this field is RO.
    #[bits(4)]
    cid_bits: u8,
    /// Collection ID Limit.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 ITS supports 16-bit Collection ID, GITS_TYPER.CIDbits is RES0.
    ///
    /// 0b1 GITS_TYPER.CIDbits indicates supported Collection ID size
    ///
    /// In implementations that do not support Collections in external memory, this bit is RES0 and the
    /// number of Collections supported is reported by GITS_TYPER.HCC.
    ///
    /// Access to this field is RO.
    cil: bool,
    vmovp: bool,
    mpam: bool,
    vsgi: bool,
    vmapp: bool,
    #[bits(2)]
    svpet: u8,
    nid: bool,
    /// Indicates support for reporting receipt of unmapped MSIs.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 Reporting of unmapped MSIs is not supported.
    ///
    /// 0b1 Reporting of unmapped MSIs is supported.
    ///
    /// Access to this field is RO.
    umsi: bool,
    /// Indicates support for generating an interrupt on receiving unmapped MSI.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 Interrupt on unmapped MSI not supported.
    ///
    /// 0b1 Interrupt on unmapped MSI is supported.
    /// If GITS_TYPER.UMSI is 0, this field is RES0.
    ///
    /// Access to this field is RO.
    umsi_irq: bool,
    inv: bool,
    #[bits(17)]
    __: (),
}

impl GITSTyper {
    pub fn get_ptr() -> *mut Self {
        (*GICITS_BASE + 0x008).into_ptr()
    }

    pub fn read() -> Self {
        unsafe { *Self::get_ptr() }
    }
}

lazy_static! {
    static ref GITS_TYPER: GITSTyper = GITSTyper::read();
}

/// Returns `rdbase` (GICR) according to GITS_TYPER.pta
/// if GITS_TYPER.pta == 0b1 returns the GICR base address
/// if GITS_TYPER.pta == 0b0 returns the GICR processor number
pub fn rdbase() -> usize {
    if GITS_TYPER.pta_base_addr() {
        GICR_BASE.into_raw()
    } else {
        0
    }
}

#[bitfield(u64)]
pub struct GITSBaser {
    /// The number of pages of physical memory allocated to the table, minus one.
    ///
    /// GITS_BASER<n>.Page_Size specifies the size of each page.
    ///
    /// If GITS_BASER<n>.Type == 0, this field is RAZ/WI.
    size: u8,
    #[bits(2)]
    /// The size of page that the table uses:
    ///
    /// 0b00 4KB.
    ///
    /// 0b01 16KB
    ///
    /// 0b10 64KB
    ///
    /// 0b11 Reserved. Treated as 0b10.
    page_size: u8,
    #[bits(2)]
    /// Indicates the Shareability attributes of accesses to the table. The possible values of this field are:
    /// 0b00 Non-shareable.
    ///
    /// 0b01 Inner Shareable.
    ///
    /// 0b10 Outer Shareable.
    ///
    /// 0b11 Reserved. Treated as 0b00.
    shareability: u8,
    #[bits(36)]
    /// Physical Address.
    /// When Page_Size is 4KB or 16KB:
    ///
    /// - Bits [51:48] of the base physical address are zero.
    ///
    /// - **This field provides bits\[47:12]** of the base physical address of the table.
    ///
    /// - Bits [11:0] of the base physical address are zero.
    ///
    /// **The address must be aligned to the size specified in the Page Size field. Otherwise the effect
    /// is CONSTRAINED UNPREDICTABLE, and can be one of the following:**
    ///
    /// - Bits[X:12], where X is derived from the page size, are treated as zero.

    /// - The value of bits[X:12] are used when calculating the address of a table access.
    ///
    /// When Page_Size is 64KB:
    ///
    /// - Bits[47:16] of the register provide bits[47:16] of the base physical address of the table.
    ///
    /// - Bits[15:12] of the register provide bits[51:48] of the base physical address of the table.
    ///
    /// - Bits[15:0] of the base physical address are 0.
    ///
    /// In implementations that support fewer than 52 bits of physical address, any unimplemented upper
    /// bits might be RAZ/WI.
    phys_addr: usize,
    #[bits(5)]
    /// Read-only. Specifies the number of bytes per table entry, minus one.
    entry_size: u8,
    #[bits(3)]
    outer_cache: u8,
    #[bits(3)]
    /// Read only. Specifies the type of entity that requires entries in the corresponding table. The possible
    /// values of the field are:
    ///
    /// 0b000 Unimplemented. This register does not correspond to an ITS table.
    ///
    /// 0b001 Devices. This register corresponds to an ITS table that scales with the width of the
    /// DeviceID. Only a single GITS_BASER<n> register reports this type.
    ///
    /// 0b010 vPEs. FEAT_GICv4 only. This register corresponds to an ITS table that scales with the
    /// number of vPEs in the system. The table requires (ENTRY_SIZE * N) bytes of memory,
    ///
    /// where N is the number of vPEs in the system. Only a single GITS_BASER<n> register
    /// reports this type.
    ///
    /// 0b100 Interrupt collections. This register corresponds to an ITS table that scales with the
    /// number of interrupt collections in the system. The table requires (ENTRY_SIZE * N)
    /// bytes of memory, where N is the number of interrupt collections. Not more than one
    /// GITS_BASER<n> register will report this type
    ty: u8,
    #[bits(3)]
    inner_cache: u8,
    /// This field indicates whether an implemented register specifies a single, flat table or a two-level table
    ///
    /// where the first level contains a list of descriptors.
    ///
    /// 0b0 Single Level. The Size field indicates the number of pages used by the ITS to store data
    /// associated with each table entry.
    ///
    /// 0b1 Two Level. The Size field indicates the number of pages which contain an array of
    /// 64-bit descriptors to pages that are used to store the data associated with each table
    /// entry. A little endian memory order model is used.
    indirect: bool,
    /// Indicates whether software has allocated memory for the table:
    ///
    /// 0b0 No memory is allocated for the table. The ITS discards any writes to the interrupt
    /// translation page when either:
    ///
    /// • GITS_BASER<n>.Type specifies any valid table entry type other than interrupt
    /// collections, that is, any value other than 0b100.
    ///
    /// • GITS_BASER<n>.Type specifies an interrupt collection and
    /// GITS_TYPER.HCC == 0.
    ///
    /// 0b1 Memory is allocated to the table
    valid: bool,
}

impl GITSBaser {
    fn get_ptr(n: usize) -> *mut Self {
        (*GICITS_BASE + (0x0100) + (8 * n)).into_ptr::<Self>()
    }

    fn read(n: usize) -> Self {
        unsafe { *Self::get_ptr(n) }
    }

    unsafe fn write(self, n: usize) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(n), self);
        }
    }

    /// Maps and initializes a GITS_BASER<N> where N = `n`
    unsafe fn setup(n: usize) -> (VirtAddr, usize) {
        let baser = GITSBaser::read(n);

        let entry_size = baser.entry_size() + 1;

        let its_page_size = match baser.page_size() {
            0b00 => 4,
            0b01 => 16,
            0b10 | 0b11 => 64,
            _ => unreachable!(),
        };

        // Y its pages = X system pages (Y / 4)
        let page_size = its_page_size / (PAGE_SIZE / SIZE_1K);
        let (devices_start, _) = frame_allocator::allocate_contiguous(page_size, page_size)
            .expect("failed to allocate space for the its device collection");

        let devices_size = page_size * PAGE_SIZE;
        unsafe {
            let devices_table = core::slice::from_raw_parts_mut(
                devices_start.into_ptr::<u8>().as_ptr(),
                devices_size,
            );
            devices_table.fill(0);
            GITSBaser::new()
                .with_ty(0b1)
                .with_entry_size(entry_size - 1)
                .with_page_size(baser.page_size())
                .with_valid(true)
                // allocated 1 its pages sizes worth
                .with_size(0)
                .with_phys_addr(
                    devices_start.start_address().into_raw() >> 12, /*
                                                                        if the page_size is 4KiB or 16KiB then this represents bits 12:47 of the addr otherwise,
                                                                        bits 16:47 of the register resperesents bits 16:47 of the addr, the offset of this field is 12
                                                                    */
                )
                .write(n);

            let addr = VirtAddr::from_ptr(devices_table.as_ptr());
            let size = devices_size;
            (addr, size)
        }
    }
}

#[bitfield(u64)]
pub struct GITSCBaser {
    /// The number of 4KB pages of physical memory allocated to the command queue, minus one.
    size: u8,
    #[bits(2)]
    __: (),
    #[bits(2)]
    /// Indicates the Shareability attributes of accesses to the table. The possible values of this field are:
    /// 0b00 Non-shareable.
    ///
    /// 0b01 Inner Shareable.
    ///
    /// 0b10 Outer Shareable.
    ///
    /// 0b11 Reserved. Treated as 0b00.
    shareability: u8,
    #[bits(40)]
    /// **Bits [51:12]** of the base physical address of the command queue.
    /// Bits [11:0] of the base address are 0.
    ///
    /// In implementations supporting fewer than 52 bits of physical address, unimplemented upper bits are
    /// RES0.
    ///
    /// If bits [15:12] are not all zeros, behavior is a CONSTRAINED UNPREDICTABLE choice:
    ///
    /// • Bits [15:12] are treated as if all the bits are zero. The value read back from those bits is either
    /// the value written or zero.
    ///
    /// • The result of the calculation of an address for a command queue read can be corrupted.
    phys_addr: usize,
    #[bits(1)]
    __: (),
    #[bits(3)]
    outer_cache: u8,
    #[bits(3)]
    __: (),
    #[bits(3)]
    inner_cache: u8,
    #[bits(1)]
    __: (),
    /// Indicates whether software has allocated memory for the command queue:
    ///
    /// 0b0 No memory is allocated for the command queue.
    ///
    /// 0b1 Memory is allocated to the command queue.
    valid: bool,
}

impl GITSCBaser {
    fn get_ptr() -> *mut Self {
        (*GICITS_BASE + 0x0080).into_ptr::<Self>()
    }

    fn read() -> Self {
        unsafe { *Self::get_ptr() }
    }

    unsafe fn write(self) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(), self);
        }
    }
}

/// Allocates an ITT,
/// returns it's base address, its size, its ITT range
pub fn allocate_itt() -> (VirtAddr, usize, u8) {
    let event_id_bits = GITS_TYPER.event_id_bits() + 1;
    let itt_entry_size = GITS_TYPER.itt_entry_size() + 1;
    /*
    * An ITT must be assigned a contiguous physical address space starting at ITT Address. The size is 2^(DTE.ITT
    Range + 1)* GITS_TYPER.ITT_entry_size
    */
    let size = itt_entry_size as usize * event_id_bits as usize;
    let pages = align_up(size, PAGE_SIZE).div_ceil(PAGE_SIZE);
    let (start_frame, _) =
        frame_allocator::allocate_contiguous(1, pages).expect("failed to allocate an ITT");

    (start_frame.virt_addr(), size, event_id_bits)
}

fn map_command_quene() -> (VirtAddr, usize) {
    let frame = frame_allocator::allocate_aligned(SIZE_64K_PAGES)
        .expect("failed to allocate a 64KiB aligned page for ITS command queue");
    unsafe {
        let addr = frame.virt_addr();
        let phys_addr = frame.phys_addr();
        let size = PAGE_SIZE;

        GITSCBaser::new()
            .with_size((size / PAGE_SIZE) as u8 - 1)
            .with_phys_addr(phys_addr.into_raw() >> 12)
            .with_valid(true)
            .write();

        debug!(
            GITSCBaser,
            "mapped command queue at {frame:?}, {:?}",
            GITSCBaser::read()
        );
        (addr, size)
    }
}

fn map_devices_table() -> (VirtAddr, usize) {
    for n in 0..3 {
        let baser = GITSBaser::read(n);
        match baser.ty() {
            0b1 => return unsafe { GITSBaser::setup(n) },
            _ => continue,
        }
    }

    unreachable!("no device's table found for GITS at {:?}", *GICITS_BASE)
}

fn map_collections_table() -> (VirtAddr, usize) {
    for n in 0..3 {
        let baser = GITSBaser::read(n);
        match baser.ty() {
            0b100 => return unsafe { GITSBaser::setup(n) },
            _ => continue,
        }
    }

    unreachable!(
        "no interrupt's collection table found for GITS at {:?}",
        *GICITS_BASE
    )
}

#[bitfield(u32)]
pub struct GITSCtlr {
    /// Controls whether the ITS is enabled:
    ///
    /// 0b0 The ITS is not enabled. Writes to GITS_TRANSLATER are ignored and no further
    /// command queue entries are processed.
    ///
    /// 0b1 The ITS is enabled. Writes to GITS_TRANSLATER result in interrupt translations and
    /// the command queue is processed.
    enabled: bool,
    #[bits(7)]
    __: (),
    /// Unmapped MSI reporting interrupt enable.
    ///
    /// 0b0 The ITS does not assert an interrupt signal when GITS_STATUSR.UMSI is 1.
    ///
    /// 0b1 The ITS asserts an interrupt signal when GITS_STATUSR.UMSI is 1.
    ///
    /// If GITS_TYPER.UMSIirq is 0, this field is RES0
    umsi_irq: bool,
    #[bits(22)]
    __: (),

    /// Read-only. Indicates completion of all ITS operations when GITS_CTLR.Enabled == 0.
    ///
    /// 0b0 The ITS is not quiescent and cannot be powered down.
    ///
    /// 0b1 The ITS is quiescent and can be powered down.
    ///
    /// For the ITS to be considered inactive, there must be no transactions in progress. In addition, all
    /// operations required to ensure that mapping data is consistent with external memory must be
    /// complete.
    quiescent: bool,
}

impl GITSCtlr {
    fn get_ptr() -> *mut Self {
        GICITS_BASE.into_ptr::<Self>()
    }

    unsafe fn write(self) {
        unsafe { core::ptr::write_volatile(Self::get_ptr(), self) }
    }
}

pub fn gits_translater() -> *mut u32 {
    (*GICITS_TRANSLATION_BASE + 0x0040).into_ptr()
}

pub fn init() {
    debug!(GITSTyper, "{:#?}", *GITS_TYPER);

    let (addr, size) = map_devices_table();
    debug!(
        GITSBaser,
        "initialized devices table at {addr:?}, with size {size:#x} bytes"
    );

    let (addr, size) = map_collections_table();
    debug!(
        GITSBaser,
        "initialized collections table at {addr:?}, with size {size:#x} bytes"
    );

    unsafe {
        GITS_COMMAND_QUEUE.lock().init();

        GITSCtlr::new().with_enabled(true).write();

        GITS_COMMAND_QUEUE
            .lock()
            .add_command(ITSCommand::new_mapc(0, rdbase(), true));
    }
}
