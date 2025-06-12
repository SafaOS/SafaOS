use crate::{
    arch::pci::{build_msi_addr, build_msi_data},
    debug,
    drivers::{
        interrupts::{IRQInfo, IntTrigger},
        pci::extended_caps::ExtendedCaptability,
    },
    write_ref, PhysAddr,
};

use super::extended_caps::GenericCaptability;
use bitfield_struct::bitfield;

#[bitfield(u16)]
struct MSIXMsgCtrl {
    #[bits(11)]
    table_size: usize,
    #[bits(3)]
    __: (),
    #[bits(1)]
    func_mask: bool,
    #[bits(1)]
    enable: bool,
}

#[bitfield(u32)]
struct Reg {
    #[bits(3)]
    bir: usize,
    #[bits(29)]
    off: u32,
}

#[repr(C)]
#[derive(Debug)]
pub struct MSIXCap {
    header: GenericCaptability,
    msg_ctrl: MSIXMsgCtrl,
    table: Reg,
    pending_bit: Reg,
}

impl ExtendedCaptability for MSIXCap {
    fn id() -> u8 {
        0x11
    }
    fn header(&self) -> &GenericCaptability {
        &self.header
    }
}

#[derive(Debug)]
#[repr(C)]
struct MSIXTableEntry {
    msg_addr: PhysAddr,
    msg_data: u32,
    vector_control: u32,
}

#[derive(Debug, Clone)]
pub struct MSIXInfo {
    cap_ptr: *mut MSIXCap,
    table_base_addr: PhysAddr,
    pab_base_addr: PhysAddr,
    table_size: usize,
    next_vector: u8,
    device_id: u16,
    vendor_id: u16,
    requester_id: u32,
}

impl MSIXInfo {
    #[allow(unused)]
    pub const fn requester_id(&self) -> u32 {
        self.requester_id
    }

    pub fn new(
        cap_ptr: *mut MSIXCap,
        device_id: u16,
        vendor_id: u16,
        requester_id: u32,
        bars: &[(PhysAddr, usize)],
    ) -> Self {
        let msix_cap = unsafe { &mut *cap_ptr };
        let table_bar = msix_cap.table.bir();
        let table_off = msix_cap.table.off() << 3;

        let pending_bit_bar = msix_cap.pending_bit.bir();
        let pending_bit_off = msix_cap.pending_bit.off() << 3;
        assert!(
            table_bar < bars.len(),
            "table bar index is {table_bar}, while bars.len() is {}, bars: {bars:?}",
            bars.len()
        );

        let table_base_addr = bars[table_bar].0 + table_off as usize;
        let pab_base_addr = bars[pending_bit_bar].0 + pending_bit_off as usize;

        assert!(table_base_addr < bars[table_bar].0 + bars[table_bar].1);
        assert!(pab_base_addr < bars[pending_bit_bar].0 + bars[pending_bit_bar].1);

        let table_size = msix_cap.msg_ctrl.table_size();

        Self {
            cap_ptr,
            table_base_addr,
            pab_base_addr,
            table_size,
            device_id,
            vendor_id,
            requester_id,
            next_vector: 0,
        }
    }

    fn table_ptr(&self) -> *mut MSIXTableEntry {
        self.table_base_addr
            .into_virt()
            .into_ptr::<MSIXTableEntry>()
    }

    fn table_entry_ptrs(&mut self, vector: u8) -> (*mut PhysAddr, *mut u32, *mut u32) {
        assert!((vector as usize) < self.table_size + 1);
        let ptr = self.table_ptr();

        unsafe {
            let base_ptr = ptr.add(vector as usize);
            let msg_addr_ptr = base_ptr as *mut PhysAddr;
            let msg_data_ptr = msg_addr_ptr.add(1) as *mut u32;
            let vector_ctrl_ptr = msg_data_ptr.add(1);
            (msg_addr_ptr, msg_data_ptr, vector_ctrl_ptr)
        }
    }

    fn write_table_entry(&mut self, vector: u8, entry: MSIXTableEntry) {
        unsafe {
            let (msg_addr_ptr, msg_data_ptr, vector_ctrl_ptr) = self.table_entry_ptrs(vector);
            core::ptr::write_volatile(msg_addr_ptr, entry.msg_addr);
            core::ptr::write_volatile(msg_data_ptr, entry.msg_data);
            core::ptr::write_volatile(vector_ctrl_ptr, entry.vector_control);
        }
    }

    fn clear_pending_interrupts(&mut self, vector: u8) {
        let pba_ptr = self.pab_base_addr.into_virt().into_ptr::<u32>();
        let vector = vector as u8;
        let byte_off = vector / 32;
        let bit_off = vector % 32;

        let byte_ptr = unsafe { pba_ptr.add(byte_off as usize) };
        unsafe {
            core::ptr::write_volatile(byte_ptr, *byte_ptr & !(1 << bit_off));
        }
    }

    /// Setups and enables MSI-X
    pub fn setup(&mut self, irq_num: u32, trigger: IntTrigger) {
        let vector = self.next_vector;
        let msix_cap = unsafe { &mut *self.cap_ptr };

        // Disable MSI-X before doing anything
        let msg = msix_cap.msg_ctrl;
        write_ref!(msix_cap.msg_ctrl, msg.with_enable(false));

        let msi_msg_addr = build_msi_addr();
        let msi_msg_data = build_msi_data(irq_num, trigger);
        let msi_table_entry = MSIXTableEntry {
            msg_addr: msi_msg_addr,
            msg_data: msi_msg_data,
            vector_control: 0,
        };

        self.write_table_entry(vector, msi_table_entry);

        // Enable MSI-X
        let msg = msix_cap.msg_ctrl;
        write_ref!(msix_cap.msg_ctrl, msg.with_enable(true));

        self.clear_pending_interrupts(vector);
        debug!(MSIXInfo, "enabled MSI-X for device id {:#x} with vendor id {:#x}: {:#x?}, table base: {:?}, pba base: {:?}", self.device_id, self.vendor_id, msix_cap, self.table_base_addr, self.pab_base_addr);
    }

    pub const fn into_irq_info(self) -> IRQInfo {
        IRQInfo::MSIX(self)
    }
}
