use bitfield_struct::bitfield;
use lazy_static::lazy_static;

use crate::{
    arch::aarch64::gic::GICR_BASE,
    memory::{
        frame_allocator::{self, SIZE_64K_PAGES},
        paging::PAGE_SIZE,
    },
    utils::locks::Mutex,
    PhysAddr,
};

#[bitfield(u8)]
pub struct LPIConfEntry {
    enable: bool,
    #[bits(1)]
    __: (),
    #[bits(6)]
    priority: u8,
}

// TODO: docs
#[bitfield(u64)]
pub struct GICRPropBaser {
    #[bits(5)]
    id_bits: u8,
    #[bits(2)]
    __: (),
    #[bits(3)]
    inner_cache: u8,
    #[bits(2)]
    sharability: u8,
    #[bits(40)]
    physical_address: usize,
    #[bits(4)]
    __: (),
    #[bits(3)]
    outer_cache: u8,
    #[bits(5)]
    __: (),
}

impl GICRPropBaser {
    pub fn get_ptr() -> *mut Self {
        (*GICR_BASE + 0x0070).into_ptr()
    }

    pub fn read() -> Self {
        unsafe { Self::get_ptr().read() }
    }

    pub unsafe fn write(self) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(), self);
        }
    }
}

// TODO: docs
#[bitfield(u64)]
pub struct GICRPendBaser {
    #[bits(7)]
    __: (),
    #[bits(3)]
    inner_cache: u8,
    #[bits(2)]
    sharebility: u8,
    #[bits(4)]
    __: (),
    #[bits(36)]
    physical_address: usize,
    #[bits(4)]
    __: (),
    #[bits(3)]
    outer_cache: u8,
    #[bits(3)]
    __: (),
    /// Pending Table Zero. Indicates to the Redistributor whether the LPI Pending table is zero when
    /// GICR_CTLR.EnableLPIs == 1.
    ///
    /// This field is WO, and reads as 0.
    ///
    /// 0b0 The LPI Pending table is not zero, and contains live data.
    ///
    /// 0b1 The LPI Pending table is zero. Software must ensure the LPI Pending table is zero
    /// before this value is written.
    ptz: bool,
    #[bits(1)]
    __: (),
}

impl GICRPendBaser {
    pub fn get_ptr() -> *mut Self {
        (*GICR_BASE + 0x0078).into_ptr()
    }

    pub fn read() -> Self {
        unsafe { Self::get_ptr().read() }
    }

    pub unsafe fn write(self) {
        unsafe {
            core::ptr::write_volatile(Self::get_ptr(), self);
        }
    }
}

pub struct LPIManager {
    configuration_table: *mut [LPIConfEntry],
    configuration_table_base: PhysAddr,
    pending_table: *mut [u8],
    pending_table_base: PhysAddr,
    id_bits: u8,
}

impl LPIManager {
    pub fn new() -> Self {
        // Allocates the LPI configuration table
        // Size in bytes = 2^(GICD_TYPER.IDbits+1) – 8192
        // FIXME: assumes GICD_TYPER.IDBits = 15
        let id_bits = 16u8;
        let conf_t_size = 2usize.pow(id_bits as u32) - 8192;
        // each entry is 1 byte or 8 bits
        let conf_t_len = conf_t_size / size_of::<LPIConfEntry>();

        let (conf_start_frame, _) = frame_allocator::allocate_contiguous(
            1,
            conf_t_size.next_multiple_of(PAGE_SIZE).div_ceil(PAGE_SIZE),
        )
        .expect("failed to allocate space for the LPI configuration table");

        let lpi_configuration_table = unsafe {
            let ptr: *mut LPIConfEntry = conf_start_frame.into_ptr::<LPIConfEntry>().as_ptr();

            let slice = core::slice::from_raw_parts_mut(ptr, conf_t_len);
            slice.fill(core::mem::zeroed());

            slice
        };

        let pending_t_size = (2usize.pow(id_bits as u32) - 8192) / 8;
        let pending_t_len = pending_t_size;

        crate::serial!(
            "num pages: {}\n",
            pending_t_size
                .next_multiple_of(PAGE_SIZE)
                .div_ceil(PAGE_SIZE)
        );
        let (pending_t_start_frame, _) = frame_allocator::allocate_contiguous(
            SIZE_64K_PAGES,
            pending_t_size
                .next_multiple_of(PAGE_SIZE)
                .div_ceil(PAGE_SIZE),
        )
        .expect("failed to allocate space for the pending table");

        let lpi_pending_table = unsafe {
            let ptr: *mut u8 = pending_t_start_frame.into_ptr::<u8>().as_ptr();
            let slice = core::slice::from_raw_parts_mut(ptr, pending_t_len);

            slice.fill(0);
            slice
        };

        Self {
            configuration_table: lpi_configuration_table,
            configuration_table_base: conf_start_frame.phys_addr(),
            pending_table: lpi_pending_table,
            pending_table_base: pending_t_start_frame.phys_addr(),
            id_bits,
        }
    }

    /// Initializes and writes GICR_PROPBASER and GICR_PENDBASER
    /// with this LPI manager configurations
    /// unsafe because the LPI manager has to be in a new state before use (zeroed) and before setting GICR_CTLR,enableLPIs to 1
    pub unsafe fn init(&mut self) {
        unsafe {
            GICRPropBaser::new()
                .with_id_bits(self.id_bits - 1)
                .with_physical_address(self.configuration_table_base.into_raw() >> 12)
                .write();
            GICRPendBaser::new()
                .with_ptz(true)
                .with_physical_address(self.pending_table_base.into_raw() >> 16)
                .write();
        }
    }

    fn write_conf(&mut self, index: usize, conf: LPIConfEntry) {
        assert!(index < self.configuration_table.len());
        unsafe {
            (self.configuration_table as *mut LPIConfEntry)
                .add(index)
                .write_volatile(conf);
        }
    }

    fn read_conf(&self, index: usize) -> LPIConfEntry {
        assert!(index < self.configuration_table.len());
        unsafe {
            (self.configuration_table as *mut LPIConfEntry)
                .add(index)
                .read()
        }
    }

    pub fn enable(&mut self, lpi_intid: u32) {
        assert!(lpi_intid >= 8192);
        let index = (lpi_intid - 8192) as usize;
        let conf = self.read_conf(index);
        self.write_conf(index, conf.with_enable(true));
    }
}

unsafe impl Send for LPIManager {}
unsafe impl Sync for LPIManager {}

lazy_static! {
    pub static ref LPI_MANAGER: Mutex<LPIManager> = Mutex::new(LPIManager::new());
}
