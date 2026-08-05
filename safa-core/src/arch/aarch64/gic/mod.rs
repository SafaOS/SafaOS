use core::cell::SyncUnsafeCell;

use alloc::boxed::Box;
use lazy_static::lazy_static;

use crate::{
    PhysAddr, VirtAddr,
    arch::aarch64::{
        cpu::CPUDevice,
        gic::gicr::{GICRDesc, lpis::LPIManager},
        interrupts::{CPUInterruptController, IntGroup, register_int_controller},
        registers::{ArchCpuID, MPIDR},
    },
    debug, info,
    memory::{
        AlignToPage,
        frame_allocator::SIZE_64K,
        init::heap0_hint,
        vmm::{self, Location, VMMAllocError, VMMMFlags, VirtualMemoryManager},
    },
};

use hfdt_rs as dtb;

pub mod cpu_if;
mod gicd;
mod gicr;
pub use gicr::lpis::LPI_MANAGER;
pub mod its;

pub struct GICv3ITS {
    base: PhysAddr,
    size: usize,
}

impl CPUDevice for GICv3ITS {
    const COMPATIBLE: &'static [&'static str] = &["arm,gic-v3-its"];

    fn create(node: dtb::Node) -> Result<Self, &'static str> {
        let mut reg = node
            .reg_addresses()
            .ok_or("ITS Node no <reg>")?
            .map(|(addr, size)| (PhysAddr::from(addr), size));
        let (base_addr, size) = reg.next().ok_or("ITS no base addr in <reg>")?;
        Ok(Self {
            base: base_addr,
            size,
        })
    }
}

struct GICv3 {
    gicc: Option<(PhysAddr, usize)>,
    gicd: (PhysAddr, usize),
    gicr: (PhysAddr, usize),
    node: dtb::Node<'static>,
}

impl CPUInterruptController for GICv3 {
    fn node(&self) -> &hfdt_rs::Node<'static> {
        &self.node
    }

    fn enable_dev_irq(
        &self,
        mut specifier: hfdt_rs::Cells,
        group: IntGroup,
    ) -> Result<u32, &'static str> {
        let [int_type, int_num, int_flags] = specifier
            .next_array()
            .ok_or("Failed to decode given interrupt specifier")?;

        _ = int_flags;
        let int_id = IntID::from_int_id(match int_type {
            0x1 => int_num + 0x10,
            0x0 => int_num + 0x20,
            _ => return Err("Unsupported interrupt type"),
        });

        Ok(int_id.set_group_all(group).enable_all().id)
    }
    fn send_eoi(&self, irq: u32, is_fiq: bool) {
        let int_id = IntID::from_int_id(irq);
        int_id.clear_pending().deactivate(is_fiq);
    }
}

impl CPUDevice for GICv3 {
    const COMPATIBLE: &'static [&'static str] = &["arm,gic-v3"];
    fn create(node: dtb::Node<'static>) -> Result<Self, &'static str> {
        let mut reg = node
            .reg_addresses()
            .ok_or("Node missing reg")?
            .map(|(addr, size)| (PhysAddr::from(addr), size));
        let gicd = reg.next().ok_or("Expected GICD Address in <reg>")?;
        let gicr = reg.next().ok_or("Expected GICR Address in <reg>")?;
        let gicc = reg.next();

        Ok(Self {
            gicc,
            gicd,
            gicr,
            node,
        })
    }
}
lazy_static! {
    static ref GICITS: GICv3ITS = GICv3ITS::lookup().expect("Cannot operate without GICv3 ITS");
    static ref GICV3: GICv3 = GICv3::lookup().expect("GICv3 not found cannot operate.");
    static ref GICC: Option<(PhysAddr, usize)> = GICV3.gicc.map(|(base, size)| (base, size));
    static ref GICD: (PhysAddr, usize) = {
        let (base, size) = GICV3.gicd;
        (base, size)
    };
    static ref GICR: (PhysAddr, usize) = {
        let (base, size) = GICV3.gicr;
        (base, size)
    };
    static ref GICD_SIZE: usize = GICD.1;
    static ref GICR_SIZE: usize = GICR.1;
    static ref GICITS_SIZE: usize = GICITS.size;
    static ref GICITS_TRANSLATION_BASE: VirtAddr = unsafe { *GICITS_BASE.get() + 0x010000 };
    static ref GICITS_TRANSLATION_BASE_PHYS: PhysAddr = *GICITS_BASE_PHYS + 0x010000;
    static ref GICITS_BASE_PHYS: PhysAddr = GICITS.base;
    static ref SGI_BASE: VirtAddr = unsafe { *GICR_BASE.get() + SIZE_64K };
}

static GICC_BASE: SyncUnsafeCell<VirtAddr> = SyncUnsafeCell::new(VirtAddr::null());
static GICD_BASE: SyncUnsafeCell<VirtAddr> = SyncUnsafeCell::new(VirtAddr::null());
static GICR_BASE: SyncUnsafeCell<VirtAddr> = SyncUnsafeCell::new(VirtAddr::null());
static GICITS_BASE: SyncUnsafeCell<VirtAddr> = SyncUnsafeCell::new(VirtAddr::null());

unsafe fn map_gic(vmm: &VirtualMemoryManager) -> Result<(), VMMAllocError> {
    unsafe {
        let flags = VMMMFlags::WRITABLE | VMMMFlags::UNCACHABLE;
        let hint = heap0_hint() + (1024 * 1024 * 1024 * 1024);

        if let Some((gicc_base, size)) = *GICC {
            *GICC_BASE.get() = vmm.map_direct_phys(
                &"GICC",
                Some(Location::Hint(hint)),
                gicc_base,
                size.to_next_page(),
                flags,
            )?;
        }

        let (gicd_pbase, gicd_size) = *GICD;
        *GICD_BASE.get() = vmm.map_direct_phys(
            &"GICD",
            Some(Location::Hint(hint)),
            gicd_pbase,
            gicd_size.to_next_page(),
            flags,
        )?;

        let (gicr_pbase, gicr_size) = *GICR;
        *GICR_BASE.get() = vmm.map_direct_phys(
            &"GICR",
            Some(Location::Hint(hint)),
            gicr_pbase,
            gicr_size.to_next_page(),
            flags,
        )?;

        let (gicits_pbase, gicits_size) = (GICITS.base, GICITS.size);
        *GICITS_BASE.get() = vmm.map_direct_phys(
            &"GICITS",
            Some(Location::Hint(hint)),
            gicits_pbase,
            gicits_size.to_next_page(),
            flags,
        )?;

        Ok(())
    }
}

lazy_static! {
    static ref GICR_DESCRIPTORS: Box<[gicr::GICRDesc]> =
        unsafe { gicr::GICRDesc::get_all_from_base(*GICR_BASE.get()) }.into_boxed_slice();
}

pub fn gic_init_cpu() {
    cpu_if::init();
}

pub fn init_gic() {
    unsafe {
        vmm::with_root(|vmm| {
            map_gic(vmm).expect("failed to map gic");
        });
    }

    register_int_controller(&*GICV3);
    unsafe {
        info!(
            "initializing GIC GICD: {:?}, GICR: {:?}",
            *GICD_BASE.get(),
            *GICR_BASE.get()
        );
    }

    gicd::init();
    for gicr in &*GICR_DESCRIPTORS {
        let enable_lpis = gicr.is_root();
        gicr.init(enable_lpis);
    }

    cpu_if::init();
    its::init();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        mut do_with_reg: impl FnMut(*mut T),
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

    /// Requests self assuming it is a sgi, if it isn't will panic
    pub fn request_sgi_all(&self, is_group0: bool) {
        assert_eq!(self.kind, IntKind::SGI);
        cpu_if::send_sgi(
            is_group0,
            self.id() as u8,
            ArchCpuID::construct(0, 0, 0, 0),
            true,
        );
    }
}
