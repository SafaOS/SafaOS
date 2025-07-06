use alloc::boxed::Box;
use lazy_static::lazy_static;

use crate::{
    VirtAddr,
    arch::{
        aarch64::{
            cpu,
            gic::gicr::{GICRDesc, lpis::LPIManager},
            registers::MPIDR,
        },
        paging::current_higher_root_table,
    },
    debug, info,
    memory::{
        frame_allocator::SIZE_64K,
        paging::{EntryFlags, MapToError, PAGE_SIZE},
    },
};

use super::paging::PageTable;

pub mod cpu_if;
mod gicd;
mod gicr;
pub use gicr::lpis::LPI_MANAGER;
pub mod its;

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
    static ref GICITS: (VirtAddr, usize) = {
        let (base, size) = *cpu::GICITS;
        (base.into_virt(), size)
    };
    static ref GICD_BASE: VirtAddr = GICD.0;
    static ref GICD_SIZE: usize = GICD.1;
    static ref GICR_BASE: VirtAddr = GICR.0;
    static ref GICR_SIZE: usize = GICR.1;
    static ref SGI_BASE: VirtAddr = *GICR_BASE + SIZE_64K;
    static ref GICITS_BASE: VirtAddr = GICITS.0;
    static ref GICITS_SIZE: usize = GICITS.1;
    static ref GICITS_TRANSLATION_BASE: VirtAddr = *GICITS_BASE + 0x010000;
}

unsafe fn map_gic(dest: &mut PageTable) -> Result<(), MapToError> {
    unsafe {
        let flags = EntryFlags::WRITE | EntryFlags::DEVICE_UNCACHEABLE;
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
        dest.map_contiguous_pages(
            *GICITS_BASE,
            (*GICITS_BASE).into_phys(),
            (*GICITS_SIZE).div_ceil(PAGE_SIZE),
            flags,
        )?;
        Ok(())
    }
}

lazy_static! {
    static ref GICR_DESCRIPTORS: Box<[gicr::GICRDesc]> =
        unsafe { gicr::GICRDesc::get_all_from_base(*GICR_BASE) }.into_boxed_slice();
}
pub fn init_gic() {
    unsafe {
        map_gic(&mut *current_higher_root_table()).expect("failed to map gic");
    }
    info!(
        "initializing GIC GICD: {:?}, GICR: {:?}",
        *GICD_BASE, *GICR_BASE
    );

    gicd::init();
    for gicr in &*GICR_DESCRIPTORS {
        let enable_lpis = gicr.is_root();
        gicr.init(enable_lpis);
    }

    extern "C" fn init_cpu(_: usize) {
        cpu_if::init();
    }

    super::arch_utils::parked_cpus_do(init_cpu, 0);
    cpu_if::init();
    its::init();
}

#[derive(Debug, Clone, Copy)]
pub enum IntKind {
    /// handled by the GICR
    PPI,
    /// handled by the GICR
    SGI,
    /// handled by the GICD
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
}

/// The interrupt group
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntGroup {
    #[allow(unused)]
    /// Group 0, Typically used by FIQs
    Secure = 0,
    /// Group 1, Typically used by IRQs
    NonSecure = 1,
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

    unsafe fn do_all_generic_set<const ALL_CPUS: bool, T: From<u32>>(
        &self,
        get_gicd_reg: impl Fn() -> *mut T,
        get_gicr_reg: impl Fn(&GICRDesc) -> *mut T,
        lpi_manager_do: impl Fn(&mut LPIManager),
    ) {
        let interrupt = self.id;
        let value = interrupt % (size_of::<T>() * 8) as u32;
        let index = (interrupt / (size_of::<T>() * 8) as u32) as usize;
        unsafe {
            self.do_all_generic_custom::<ALL_CPUS, T>(
                get_gicd_reg,
                get_gicr_reg,
                |reg| reg.add(index).write_volatile((1 << value).into()),
                lpi_manager_do,
            );
        }
    }

    unsafe fn do_all_generic_custom<const ALL_CPUS: bool, T>(
        &self,
        get_gicd_reg: impl Fn() -> *mut T,
        get_gicr_reg: impl Fn(&GICRDesc) -> *mut T,
        do_with_reg: impl Fn(*mut T),
        lpi_manager_do: impl Fn(&mut LPIManager),
    ) {
        match self.kind {
            IntKind::SGI | IntKind::PPI => {
                if ALL_CPUS {
                    for gicr in &*GICR_DESCRIPTORS {
                        let reg = get_gicr_reg(gicr);
                        crate::serial!("got reg: {reg:?}\n");
                        do_with_reg(reg);
                    }
                } else {
                    let mpidr = MPIDR::read();
                    let cpu_id = mpidr.cpuid();
                    for gicr in &*GICR_DESCRIPTORS {
                        if gicr.cpu_id() == cpu_id {
                            let reg = get_gicr_reg(gicr);
                            do_with_reg(reg);
                        }
                    }
                }
            }
            IntKind::SPI => do_with_reg(get_gicd_reg()),
            IntKind::LPI => lpi_manager_do(&mut *LPI_MANAGER.lock()),
        }
    }

    /// Enables the interrupt in all CPUs
    pub fn enable_all(&self) -> &Self {
        unsafe {
            self.do_all_generic_set::<true, _>(
                || gicd::isenabler(),
                |gicr| gicr.isenabler(),
                |lpi_manager| lpi_manager.enable(self.id),
            );
        }
        debug!(IntID, "enabled interrupt with ID `{}`", self.id);
        self
    }

    fn clear_pending_generic<const ALL_CPUS: bool>(&self) -> &Self {
        unsafe {
            self.do_all_generic_set::<ALL_CPUS, _>(
                || gicd::icpendr0(),
                |gicr| gicr.icpendr0(),
                |lpi_m| lpi_m.clear_pending(self.id),
            );
        }
        self
    }

    /// Marks the interrupt as not pending in the current cpu
    pub fn clear_pending(&self) -> &Self {
        self.clear_pending_generic::<false>()
    }

    /// Marks the interrupt as not pending in all CPUs
    pub fn clear_pending_all(&self) -> &Self {
        self.clear_pending_generic::<true>()
    }

    /// Makes the interrupt pending in the current cpu
    pub fn set_pending(&self) -> &Self {
        unsafe {
            self.do_all_generic_set::<false, _>(
                || gicd::ispendr0(),
                |gicr| gicr.ispendr0(),
                |_| unimplemented!("set pending isn't implemented for LPIs"),
            );
        }
        self
    }
    /// Sets the group of the interrupt to `group` in all CPUs
    pub fn set_group_all(&self, group: IntGroup) -> &Self {
        let interrupt = self.id;
        let index = (interrupt % 32) as usize;
        let value = interrupt / 32;
        let shift = 1 << value;
        unsafe {
            self.do_all_generic_custom::<true, _>(
                || gicd::igroup0(),
                |gicr| gicr.igroup0(),
                |reg| {
                    reg.add(index)
                        .write_volatile(if group == IntGroup::NonSecure {
                            *reg | shift
                        } else {
                            *reg & !shift
                        })
                },
                |_| unimplemented!("set group isn't implemented for LPIs"),
            );
        }
        debug!(
            IntID,
            "set group of interrupt with ID `{}` to {:?}", self.id, group
        );
        self
    }

    pub fn id(&self) -> u32 {
        self.id
    }
    /// Sets the interrupt status to unactive
    /// doesn't disable the interrupt
    pub fn deactivate(&self, is_group0: bool) -> &Self {
        cpu_if::deactivate_int(self.id(), is_group0);
        self
    }
}
