use core::sync::atomic::AtomicU16;

use crate::{
    PhysAddr, VirtAddr,
    arch::pci::{build_msi_addr, build_msi_data},
    debug,
    drivers::{
        interrupts::{IRQInfo, IntTrigger},
        pci::{AllocatedBar, extended_caps::ExtendedCapability},
    },
    percpu::{CpuID, CpuLocal},
    write_ref,
};

use super::extended_caps::GenericCapability;
use bitfield_struct::bitfield;

#[bitfield(u16)]
struct MSIMsgCtrl {
    enable: bool,
    #[bits(3)]
    multiple_message_capable: u8,
    #[bits(3)]
    multiple_message_enable: u8,
    bit64: bool,
    per_vector_masking: bool,
    #[bits(7)]
    __rsvd: u8,
}

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
pub struct MSICap {
    header: GenericCapability,
    msg_ctrl: MSIMsgCtrl,
    addr_low: u32,
    addr_high: u32,
    data: u16,
    __rsvd: u16,
    mask: u32,
    pending: u32,
}

impl MSICap {
    pub unsafe fn write_addr_data(&mut self, addr: PhysAddr, data: u16) {
        write_ref!(self.addr_low, addr.into_raw() as u32);
        write_ref!(self.addr_high, (addr.into_raw() as u64 >> 32) as u32);
        write_ref!(self.data, data);
    }
}

impl ExtendedCapability for MSICap {
    fn id() -> u8 {
        0x5
    }
    fn header(&self) -> &GenericCapability {
        &self.header
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct MSIXCap {
    header: GenericCapability,
    msg_ctrl: MSIXMsgCtrl,
    table: Reg,
    pending_bit: Reg,
}

impl ExtendedCapability for MSIXCap {
    fn id() -> u8 {
        0x11
    }
    fn header(&self) -> &GenericCapability {
        &self.header
    }
}

static NEXT_CPU: AtomicU16 = AtomicU16::new(0);
fn next_cpu_for_msi() -> CpuID {
    let id = NEXT_CPU.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
        % CpuLocal::get_all().len_hint() as u16;
    CpuID::from_u16(id).expect("CpuLocal::get_all returned invalid length")
}
#[derive(Debug, Clone, Copy)]
pub struct MSIInfo {
    cap_ptr: *mut MSICap,
    requester_id: u32,
}

impl MSIInfo {
    #[allow(unused)]
    pub const fn requester_id(&self) -> u32 {
        self.requester_id
    }

    pub unsafe fn new(cap_ptr: *mut MSICap, requester_id: u32) -> Self {
        Self {
            cap_ptr,
            requester_id,
        }
    }

    /// Setups and enables MSI
    pub fn setup(&mut self, irq_num: u32, trigger: IntTrigger) {
        let cpu = next_cpu_for_msi();
        let addr = build_msi_addr(cpu);
        let data = build_msi_data(irq_num, trigger);
        assert!(
            data <= u16::MAX as u32,
            "MSI data: {data:#x} is too big for basic MSI, irq: {irq_num}"
        );

        let cap = unsafe { &mut *self.cap_ptr };
        write_ref!(
            cap.msg_ctrl,
            MSIMsgCtrl::new().with_bit64(true).with_enable(true)
        );
        unsafe { cap.write_addr_data(addr, data as u16) };
    }

    pub const fn into_irq_info(self) -> IRQInfo {
        IRQInfo::MSI(self)
    }
}

#[derive(Debug)]
#[repr(C)]
struct MSIXTableEntry {
    msg_addr: PhysAddr,
    msg_data: u32,
    vector_control: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct MSIXInfo {
    cap_ptr: *mut MSIXCap,
    table_base_addr: VirtAddr,
    pab_base_addr: VirtAddr,
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
        bars: &[AllocatedBar],
    ) -> Self {
        let msix_cap = unsafe { &mut *cap_ptr };
        let table_bar_index = msix_cap.table.bir();
        let table_off = msix_cap.table.off() << 3;

        let pending_bit_bar_index = msix_cap.pending_bit.bir();
        let pending_bit_off = msix_cap.pending_bit.off() << 3;
        assert!(
            table_bar_index < bars.len(),
            "table bar index is {table_bar_index}, while bars.len() is {}, bars: {bars:?}",
            bars.len()
        );

        let table_bar = bars[table_bar_index];
        let pending_table_bar = bars[pending_bit_bar_index];

        let AllocatedBar::Memory(table_bar_base, table_bar_size) = table_bar else {
            unreachable!("MSI Table bar isn't a memory bar")
        };

        let AllocatedBar::Memory(pending_table_bar_base, pending_table_bar_size) =
            pending_table_bar
        else {
            unreachable!("MSI Pending Table bar isn't a memory bar")
        };

        let table_base_addr = table_bar_base + table_off as usize;
        let pab_base_addr = pending_table_bar_base + pending_bit_off as usize;

        assert!(table_base_addr < table_bar_base + table_bar_size);
        assert!(pab_base_addr < pending_table_bar_base + pending_table_bar_size);

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
        self.table_base_addr.into_ptr::<MSIXTableEntry>()
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
        let pba_ptr = self.pab_base_addr.into_ptr::<u32>();
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

        let msi_msg_addr = build_msi_addr(next_cpu_for_msi());
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
        debug!(
            MSIXInfo,
            "enabled MSI-X for device id {:#x} with vendor id {:#x}: {:#x?}, table base: {:?}, pba base: {:?}",
            self.device_id,
            self.vendor_id,
            msix_cap,
            self.table_base_addr,
            self.pab_base_addr
        );
    }

    pub const fn into_irq_info(self) -> IRQInfo {
        IRQInfo::MSIX(self)
    }
}
