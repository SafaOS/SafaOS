//! A wrapper around the GIC cpu interface registers, whether it is the GICC Memory Mapped registers or raw system registers
use core::arch::asm;

use bitfield_struct::bitfield;

use crate::{VirtAddr, arch::aarch64::registers::MPIDR, debug};

/// This driver uses the GICC memory mapped registers instead of AArch64 native registers when available
#[inline(always)]
fn gicc_mem() -> Option<VirtAddr> {
    super::GICC.map(|(addr, _)| addr)
}

#[bitfield(u64)]
struct ICCCtlr {
    /// Common Binary Point Register. Controls whether the same register is used for interrupt preemption
    /// of both Group 0 and Group 1 interrupts:
    ///
    /// 0b0 ICC_BPR0_EL1 determines the preemption group for Group 0 interrupts only.
    /// ICC_BPR1_EL1 determines the preemption group for Group 1 interrupts.
    ///
    /// 0b1 ICC_BPR0_EL1 determines the preemption group for both Group 0 and Group 1
    /// interrupts.
    cbpr: bool,
    /// EOI mode for the current Security state. Controls whether a write to an End of Interrupt register also
    /// deactivates the interrupt:
    ///
    /// 0b0 ICC_EOIR0_EL1 and ICC_EOIR1_EL1 provide both priority drop and interrupt
    /// deactivation functionality. Accesses to ICC_DIR_EL1 are UNPREDICTABLE.
    ///
    /// 0b1 ICC_EOIR0_EL1 and ICC_EOIR1_EL1 provide priority drop functionality only.
    /// ICC_DIR_EL1 provides interrupt deactivation functionality.
    eoi_mode: bool,
    #[bits(4)]
    __: (),
    /// Priority Mask Hint Enable. Controls whether the priority mask register is used as a hint for interrupt
    /// distribution:
    ///
    /// 0b0 Disables use of ICC_PMR_EL1 as a hint for interrupt distribution.
    ///
    /// 0b1 Enables use of ICC_PMR_EL1 as a hint for interrupt distribution.
    pmhe: bool,
    #[bits(1)]
    __: (),
    /// Priority bits. Read-only and writes are ignored. The number of priority bits implemented, minus
    /// one.
    ///
    /// An implementation that supports two Security states must implement at least 32 levels of physical
    /// priority (5 priority bits).
    ///
    /// An implementation that supports only a single Security state must implement at least 16 levels of
    /// physical priority (4 priority bits).
    #[bits(3)]
    pri_bits: u8,
    /// Identifier bits. Read-only and writes are ignored. The number of physical interrupt identifier bits
    /// supported:
    ///
    /// 0b000 16 bits.
    ///
    /// 0b001 24 bits.
    ///
    /// All other values are reserved.
    #[bits(3)]
    idbits_24: u8,
    /// SEI Support. Read-only and writes are ignored. Indicates whether the CPU interface supports local
    /// generation of SEIs:
    ///
    /// 0b0 The CPU interface logic does not support local generation of SEIs.
    ///
    /// 0b1 The CPU interface logic supports local generation of SEIs.
    seis: bool,
    /// Affinity 3 Valid. Read-only and writes are ignored. Possible values are:
    /// 0b0 The CPU interface logic only supports zero values of Affinity 3 in SGI generation
    /// System registers.
    ///
    /// 0b1 The CPU interface logic supports nonzero values of Affinity 3 in SGI generation System
    /// registers.
    af3v: bool,
    #[bits(2)]
    __: (),
    /// Range Selector Support. Possible values are:
    ///
    /// 0b0 Targeted SGIs with affinity level 0 values of 0 - 15 are supported.
    ///
    /// 0b1 Targeted SGIs with affinity level 0 values of 0 - 255 are supported.
    ///
    /// This bit is read-only
    rss: bool,
    /// Extended INTID range (read-only).
    ///
    /// 0b0 CPU interface does not support INTIDs in the range 1024..8191.
    /// • Behavior is UNPREDICTABLE if the IRI delivers an interrupt in the range 1024 to
    /// 8191 to the CPU interface.
    /// Note
    /// Arm strongly recommends that the IRI is not configured to deliver interrupts in this
    /// range to a PE that does not support them.
    /// 0b1 CPU interface supports INTIDs in the range 1024..8191
    /// • All INTIDs in the range 1024..8191 are treated as requiring deactivation
    ext_range_intid: bool,
    #[bits(44)]
    __: (),
}

impl ICCCtlr {
    pub fn get() -> Self {
        let results: u64;
        unsafe {
            asm!("mrs {}, ICC_CTLR_EL1", out(reg) results);
        }
        Self::from_bits(results)
    }

    /// Writes self into the system register ICC_CTLR_EL1
    pub fn write(self) {
        let value = self.into_bits();
        unsafe { asm!("msr ICC_CTLR_EL1, {}", in(reg) value) }
    }
}

#[bitfield(u32)]
struct GICCtrl {
    /// This Non-secure field enables the signaling of Group 1 interrupts by the CPU interface to a target PE
    enable_grp1: bool,
    #[bits(4)]
    __: (),
    /// When the signaling of FIQs by the CPU interface is disabled, this field partly controls whether the bypass FIQ signal is signaled to the PE for Group 1
    fiq_bypass_dis_grp1: bool,
    /// When the signaling of IRQs by the CPU interface is disabled, this field partly controls whether the bypass IRQ signal is signaled to the PE for Group 1
    irq_bypass_dis_grp1: bool,
    #[bits(2)]
    __: (),
    /// Controls the behavior of Non-secure accesses to GICC_EOIR, GICC_AEOIR, and GICC_DIR.
    /// 0b0 GICC_EOIR and GICC_AEOIR provide both priority drop and interrupt deactivation
    /// functionality. Accesses to GICC_DIR are UNPREDICTABLE.
    /// 0b1 GICC_EOIR and GICC_AEOIR provide priority drop functionality only. GICC_DIR
    /// provides interrupt deactivation functionality.
    non_sec_eoi_mode: bool,
    #[bits(22)]
    __: (),
}

#[inline(always)]
fn gicc_ctlr() -> Option<*mut GICCtrl> {
    gicc_mem().map(|p| p.into_ptr::<_>())
}

#[inline(always)]
fn gicc_pmr() -> Option<*mut u32> {
    gicc_mem().map(|a| (a + 0x4).into_ptr::<u32>())
}

#[inline(always)]
fn gicc_bpr() -> Option<*mut u32> {
    gicc_mem().map(|a| (a + 0x8).into_ptr::<u32>())
}

#[inline(always)]
fn gicc_iar() -> Option<*mut u32> {
    gicc_mem().map(|a| (a + 0xC).into_ptr::<u32>())
}

#[inline(always)]
fn gicc_dir() -> Option<*mut u32> {
    gicc_mem().map(|a| (a + 0x1000).into_ptr())
}

/// Makes it so only interrupts of higher priority then `priority` is handled.
///
/// The higher the value the more interrupts that are going to be allowed, 0xff is the lowest priority and allows all interrupts
pub fn set_min_priority(priority: u8) {
    unsafe {
        if let Some(pmr) = gicc_pmr() {
            core::ptr::write_volatile(pmr, priority as u32);
        } else {
            asm!("msr ICC_PMR_EL1, {:x}", in(reg) priority)
        }
    }
}

/// Defines the point at which the priority value fields split into two parts, the group priority field and
/// the subpriority field. The group priority field determines Group 0 interrupt preemption
///
/// The value of this field controls how the 8-bit interrupt priority field is split into a group priority field,
/// that determines interrupt preemption, and a subpriority field. This is done as follows:
///
/// | Binary Point Value | Group Priority Field | Subpriority Field | Field with Binary Point |
/// |--------------------|----------------------|-------------------|--------------------------|
/// | 0                  | 7:1                  | 0                 | ggggggg.s                |
/// | 1                  | 7:2                  | 1:0               | gggggg.ss                |
/// | 2                  | 7:3                  | 2:0               | ggggg.sss                |
/// | 3                  | 7:4                  | 3:0               | gggg.ssss                |
/// | 4                  | 7:5                  | 4:0               | ggg.sssss                |
/// | 5                  | 7:6                  | 5:0               | gg.ssssss                |
/// | 6                  | 7                    | 6:0               | g.sssssss                |
/// | 7                  | No preemption        | 7:0               | .ssssssss                |
pub fn set_binary_spilt_point(point: u8) {
    unsafe {
        if let Some(bpr) = gicc_bpr() {
            core::ptr::write_volatile(bpr, point as u32);
        } else {
            asm!("msr ICC_BPR0_EL1, {:x}", in(reg) point)
        }
    }
}

/// Gets the interrupt ID of the current ingoing interrupt
#[inline(always)]
pub fn get_int_id(is_group0: bool) -> u32 {
    unsafe {
        if let Some(gicc_iar) = gicc_iar() {
            (*gicc_iar) & 0xFFFFFF
        } else {
            let results: u32;
            if is_group0 {
                asm!("mrs {:x}, ICC_IAR0_EL1", out(reg) results);
            } else {
                asm!("mrs {:x}, ICC_IAR1_EL1", out(reg) results);
            }
            results & 0xFFFFFF
        }
    }
}

/// Initializes the CPU interface registers
pub fn init() {
    if let Some(ctlr) = gicc_ctlr() {
        unsafe {
            core::ptr::write_volatile(ctlr, (*ctlr).with_enable_grp1(true));
        }
    } else {
        // TODO: pritiorize register access
        unsafe {
            // enable register access
            asm!("msr ICC_SRE_EL1, {:x}", in(reg) 1);
        }

        ICCCtlr::new()
            .with_eoi_mode(false)
            .with_cbpr(true)
            .with_pmhe(true)
            .write();

        unsafe {
            // enable group 1 interrupts
            asm!("msr ICC_IGRPEN1_EL1, {:x}", in(reg) 1);
            // enable group 0 interrupts
            asm!("msr ICC_IGRPEN0_EL1, {:x}", in(reg) 1);
        }

        let icc_ctlr = ICCCtlr::get();
        debug!(
            ICCCtlr,
            "Initialized, eoi mode: {}, priority bits: {}, intID bits: {}, affinity 3 valid: {}, extended intID range: {} for CPU: {}",
            icc_ctlr.eoi_mode(),
            icc_ctlr.pri_bits() + 1,
            if icc_ctlr.idbits_24() == 1 {
                "24"
            } else {
                "16"
            },
            icc_ctlr.af3v(),
            icc_ctlr.ext_range_intid(),
            MPIDR::read().cpuid(),
        );
    }

    set_min_priority(0xff);
    set_binary_spilt_point(0);
}

pub fn deactivate_int(int_id: u32, is_group0: bool) {
    unsafe {
        if let Some(dr) = gicc_dir() {
            core::ptr::write_volatile(dr, int_id);
        } else {
            if is_group0 {
                asm!("msr ICC_EOIR0_EL1, {:x}", in(reg) int_id)
            } else {
                asm!("msr ICC_EOIR1_EL1, {:x}", in(reg) int_id)
            }
        }
    }
}
