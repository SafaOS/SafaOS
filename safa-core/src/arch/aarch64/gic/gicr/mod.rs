use alloc::vec::Vec;
use bitfield_struct::bitfield;

use crate::{
    VirtAddr,
    arch::aarch64::{
        gic::gicr::lpis::LPI_MANAGER,
        registers::{CPUID, MPIDR},
    },
    info,
    memory::frame_allocator::SIZE_64K,
};

pub mod lpis;

pub struct GICRDesc {
    base_addr: VirtAddr,
    cpu_id: CPUID,
    is_root: bool,
}

impl GICRDesc {
    pub const fn is_root(&self) -> bool {
        self.is_root
    }

    pub const fn cpu_id(&self) -> CPUID {
        self.cpu_id
    }

    /// Creates an instance of a GICR descriptor from a given base addr
    ///
    /// also returns whether or not it is the last GICR
    pub unsafe fn from_base_addr(base_addr: VirtAddr) -> (Self, bool) {
        let this_cpu = MPIDR::read().cpuid();

        // temporary instance
        let mut this = Self {
            base_addr,
            cpu_id: this_cpu,
            is_root: false,
        };

        let typer = this.typer_reg();
        this.cpu_id = typer.cpu_id();
        this.is_root = typer.processor_num() == 0;

        (this, typer.last())
    }

    /// Gets all the GICRs for each CPU from a given base GICR address
    pub unsafe fn get_all_from_base(base_addr: VirtAddr) -> Vec<Self> {
        unsafe {
            let mut base_addr = base_addr;
            let mut results = Vec::new();

            loop {
                let (gicr, is_last) = Self::from_base_addr(base_addr);

                base_addr = gicr.end_addr();
                results.push(gicr);

                if is_last {
                    break;
                }
            }

            results
        }
    }

    const fn sgi_base(&self) -> VirtAddr {
        self.base_addr + SIZE_64K
    }

    const fn end_addr(&self) -> VirtAddr {
        self.sgi_base() + SIZE_64K
    }

    const fn get_reg<T>(&self, offset: usize) -> *mut T {
        (self.base_addr + offset).into_ptr::<T>()
    }

    fn waker_reg(&self) -> *mut GICRWaker {
        self.get_reg(GICRWaker::OFFSET)
    }

    fn typer_reg(&self) -> GICRTyper {
        unsafe { *self.get_reg(GICRTyper::OFFSET) }
    }

    fn ctrl_reg(&self) -> *mut GICRCtlr {
        self.get_reg(GICRCtlr::OFFSET)
    }

    /// Pointer to the GICR_ISENABLER<0> Register
    #[inline(always)]
    pub fn isenabler(&self) -> *mut u32 {
        (self.sgi_base() + ISENABLER_SGI_OFF).into_ptr::<u32>()
    }

    /// Pointer to the GICR_ICPENDR<0> Register
    #[inline(always)]
    pub fn icpendr0(&self) -> *mut u32 {
        (self.sgi_base() + ICPENDER0_SGI_OFF).into_ptr::<u32>()
    }

    /// Pointer to the GICR_IGROUP<0> Register
    #[inline(always)]
    pub fn igroup0(&self) -> *mut u32 {
        (self.sgi_base() + IGROUP0_SGI_OFF).into_ptr::<u32>()
    }

    /// Pointer to the GICR_ISPENDR<0> Register
    #[inline(always)]
    pub fn ispendr0(&self) -> *mut u32 {
        (self.sgi_base() + ISPENDR0_SGI_OFF).into_ptr::<u32>()
    }

    /// Wakes up and initializes the GICR
    pub fn init(&self, enable_lpis: bool) {
        let gicr_waker = self.waker_reg();
        unsafe {
            gicr_waker.write_volatile(GICRWaker::new().with_processor_sleep(false));
            assert!(!gicr_waker.read_volatile().processor_sleep());
            // Polls until it wakes up
            while gicr_waker.read_volatile().children_asleep() {
                core::hint::spin_loop();
            }

            let gicr_typer = self.typer_reg();
            info!(
                "woke up the GICR, processor num: {}, is the last GICR: {}, supports direct lpis: {}",
                gicr_typer.processor_num(),
                gicr_typer.last(),
                gicr_typer.direct_lpi()
            );

            if enable_lpis {
                let gicr_ctlr = self.ctrl_reg();

                LPI_MANAGER.lock().init();
                let ctrl = gicr_ctlr.read();
                gicr_ctlr.write_volatile(ctrl.with_enable_lpis(true));
                info!("initialized LPI configuration table and pending table");
            }
        }
    }
}

#[bitfield(u32)]
pub struct GICRCtlr {
    /// In implementations where affinity routing is enabled for the Security state:
    ///
    /// 0b0 LPI support is disabled. Any doorbell interrupt generated as a result of a write to a
    /// virtual LPI register must be discarded, and any ITS translation requests or commands
    /// involving LPIs in this Redistributor are ignored.
    ///
    /// 0b1 LPI support is enabled.
    enable_lpis: bool,
    /// Clear Enable Supported.
    ///
    /// This bit is read-only.
    ///
    /// 0b0 The IRI does not indicate whether GICR_CTLR.EnableLPIs is RES1 once set.
    ///
    /// 0b1 GICR_CTLR.EnableLPIs is not RES1 once set.
    ///
    /// Implementing GICR_CTLR.EnableLPIs as programmable and not reporting GICR_CLTR.CES ==
    /// 1 is deprecated.
    ///
    /// Implementing GICR_CTLR.EnableLPIs as RES1 once set is deprecated.
    ///
    /// When GICR_CLTR.CES == 0, software cannot assume that GICR_CTLR.EnableLPIs is
    /// programmable without observing the bit being cleared.
    ces: bool,
    /// LPI invalidate registers supported.
    ///
    /// This bit is read-only.
    ///
    /// 0b0 This bit does not indicate whether the GICR_INVLPIR, GICR_INVALLR and
    ///
    /// GICR_SYNCR are implemented or not.
    ///
    /// 0b1 GICR_INVLPIR, GICR_INVALLR and GICR_SYNCR are implemented.
    /// If GICR_TYPER.DirectLPI is 1 or GICR_TYPER.RVPEI is 1, GICR_INVLPIR,
    /// GICR_INVALLR, and GICR_SYNCR are always implemented
    ir: bool,
    /// Register Write Pending. This bit indicates whether a register write for the current Security state is
    /// in progress or not.
    rwp: bool,
    #[bits(20)]
    __: (),
    /// Disable Processor selection for Group 0 interrupts. When GICR_TYPER.DPGS == 1:
    ///
    /// 0b0 A Group 0 SPI configured to use the 1 of N distribution model can select this PE, if the
    /// PE is not asleep and if Group 0 interrupts are enabled.
    ///
    /// 0b1 A Group 0 SPI configured to use the 1 of N distribution model cannot select this PE.
    dpg0: bool,
    /// Disable Processor selection for Group 1 Non-secure interrupts. When GICR_TYPER.DPGS == 1:
    ///
    /// 0b0 A Group 1 Non-secure SPI configured to use the 1 of N distribution model can select
    ///
    /// this PE, if the PE is not asleep and if Non-secure Group 1 interrupts are enabled.
    /// 0b1 A Group 1 Non-secure SPI configured to use the 1 of N distribution model cannot select
    /// this PE.
    dpg1ns: bool,
    /// Disable Processor selection for Group 1 Secure interrupts. When GICR_TYPER.DPGS == 1:
    ///
    /// 0b0 A Group 1 Secure SPI configured to use the 1 of N distribution model can select this
    /// PE, if the PE is not asleep and if Secure Group 1 interrupts are enabled.
    ///
    /// 0b1 A Group 1 Secure SPI configured to use the 1 of N distribution model cannot select this
    /// PE.
    dpg1s: bool,
    #[bits(4)]
    __: (),
    /// Upstream Write Pending. Read-only. Indicates whether all upstream writes have been
    /// communicated to the Distributor
    uwp: bool,
}

impl GICRCtlr {
    const OFFSET: usize = 0x0;
}

// TODO: docs?
#[bitfield(u64)]
pub struct GICRTyper {
    plpis: bool,
    vlpis: bool,
    dirty: bool,
    direct_lpi: bool,
    /// Indicates whether this Redistributor is the highest-numbered Redistributor in a series of contiguous
    /// Redistributor pages.
    ///
    /// The value of this field is an IMPLEMENTATION DEFINED choice of:
    ///
    /// 0b0 This Redistributor is not the highest-numbered Redistributor in a series of contiguous
    /// Redistributor pages.
    ///
    /// 0b1 This Redistributor is the highest-numbered Redistributor in a series of contiguous
    /// Redistributor pages.
    ///
    /// Access to this field is RO.
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
    /**
    The identity of the PE associated with this Redistributor.

    Bits [63:56] provide Aff3, the Affinity level 3 value for the Redistributor.

    Bits [55:48] provide Aff2, the Affinity level 2 value for the Redistributor.

    Bits [47:40] provide Aff1, the Affinity level 1 value for the Redistributor.

    Bits [39:32] provide Aff0, the Affinity level 0 value for the Redistributor.

    This field has an IMPLEMENTATION DEFINED value.
    Access to this field is RO.
    */
    af_value: u32,
}

impl GICRTyper {
    const OFFSET: usize = 0x8;
    /// Gets the cpu ID (affinity value) of the parent GICR
    pub const fn cpu_id(&self) -> CPUID {
        let af_value = self.af_value();
        let aff0 = (af_value >> 0) as u8;
        let aff1 = (af_value >> 8) as u8;
        let aff2 = (af_value >> 16) as u8;
        let aff3 = (af_value >> 24) as u8;

        CPUID::construct(aff0, aff1, aff2, aff3)
    }
}

#[bitfield(u32)]
pub struct GICRWaker {
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
    const OFFSET: usize = 0x14;
}

pub const IGROUP0_SGI_OFF: usize = 0x080;
pub const ISENABLER_SGI_OFF: usize = 0x100;
pub const ICPENDER0_SGI_OFF: usize = 0x280;
pub const ISPENDR0_SGI_OFF: usize = 0x200;
