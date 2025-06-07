use lazy_static::lazy_static;

use crate::{
    arch::{aarch64::cpu, paging::current_higher_root_table},
    debug, info,
    memory::paging::{EntryFlags, MapToError, PAGE_SIZE},
    VirtAddr,
};

use super::paging::PageTable;

pub mod cpu_if;
mod gicd;
mod gicr;
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
    static ref SGI_BASE: VirtAddr = *GICR_BASE + (/* 64 KiB */ 64 * 1024);

    static ref GICITS_BASE: VirtAddr = GICITS.0;
    static ref GICITS_SIZE: usize = GICITS.1;
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
    dest.map_contiguous_pages(
        *GICITS_BASE,
        (*GICITS_BASE).into_phys(),
        (*GICITS_SIZE).div_ceil(PAGE_SIZE),
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

    gicd::init();
    gicr::init();
    cpu_if::init();
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

/// Enables GIC interrupt
fn enable(interrupt: u32, int_kind: IntKind) {
    let value = interrupt % 32;
    let index = interrupt / 32;

    unsafe {
        let reg = int_kind.choose_reg(gicd::isenabler(), gicr::isenabler());
        core::ptr::write_volatile(reg.add(index as usize), 1 << value);
    }
}

#[inline(always)]
/// Clears pending interrupt
pub fn clear_pending(interrupt: u32, kind: IntKind) {
    let value = interrupt % 32;
    let index = interrupt / 32;
    unsafe {
        let reg = kind.choose_reg(gicd::icpendr0(), gicr::icpendr0());
        core::ptr::write_volatile(reg.add(index as usize), 1 << value);
    }
}

/// The interrupt group
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntGroup {
    /// Group 0, Typically used by FIQs
    Secure = 0,
    /// Group 1, Typically used by IRQs
    NonSecure = 1,
}

#[inline(always)]
pub fn set_group(int_id: u32, kind: IntKind, group: IntGroup) {
    let value = int_id % 32;
    let index = int_id / 32;

    unsafe {
        let reg = kind.choose_reg(gicd::igroup0(), gicr::igroup0());
        let reg = reg.add(index as usize);

        let shift = 1 << value;
        core::ptr::write_volatile(
            reg,
            if group == IntGroup::NonSecure {
                *reg | shift
            } else {
                *reg & !shift
            },
        );
    }
}

fn set_pending(int_id: u32, kind: IntKind) {
    let value = int_id % 32;
    let index = int_id / 32;
    let reg = kind.choose_reg(gicd::ispendr0(), gicr::ispendr0());
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
    /// Sets the interrupt status to unactive
    /// doesn't disable the interrupt
    pub fn deactivate(&self, is_group0: bool) -> &Self {
        cpu_if::deactivate_int(self.id(), is_group0);
        self
    }
}
