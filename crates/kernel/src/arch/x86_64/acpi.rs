use core::ptr::addr_of;

use lazy_static::lazy_static;

use crate::{PhysAddr, RSDP_ADDR, limine::HHDM};

lazy_static! {
    pub static ref PSDT_DESC: &'static dyn PTSD = get_sdt();
    pub static ref MADT_DESC: &'static MADT = MADT::get(*PSDT_DESC);
    pub static ref FADT_DESC: &'static FADT = FADT::get(*PSDT_DESC);
    pub static ref MCFG_DESC: &'static MCFG = MCFG::get(*PSDT_DESC);
}
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct RSDPDesc {
    signature: [u8; 8],
    checksum: u8,
    oemid: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    len: u32,
    xsdt_addr: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

impl RSDPDesc {
    pub fn vaildate(&self) -> bool {
        let size = size_of::<Self>();
        let byte_array = (self) as *const RSDPDesc as *const u8;
        let mut sum: usize = 0;

        for i in 0..size {
            unsafe {
                sum += *byte_array.add(i) as usize;
            };
        }

        (sum & 0xFF) == 0
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ACPIHeader {
    pub signatrue: [u8; 4],
    len: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RSDT {
    pub header: ACPIHeader,
    table: [u32; 0], // uint32_t table[];?
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct XSDT {
    pub header: ACPIHeader,
    table: [u64; 0],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MCFGEntry {
    pub physical_addr: PhysAddr,
    pub pci_sgn: u16,
    pub pci_num0: u8,
    pub pci_num1: u8,
}

#[repr(C, packed)]
#[derive(Debug)]
pub struct MCFG {
    pub header: ACPIHeader,
    _reserved: [u8; 8],
    entries: [MCFGEntry; 0],
}

#[repr(C, packed)]
#[derive(Debug)]
pub struct FADT {
    pub header: ACPIHeader,
    pub firmware_ctrl: u32,
    pub dsdt: u32,
    pub reserved: u8,
    pub preferred_pm_profile: u8,

    pub sci_int: u16,
    pub smi_cmd: u32,

    pub acpi_enable: u8,
    pub acpi_disable: u8,

    pub s4bios_req: u8,
    pub pstate_cnt: u8,

    pub pm1a_evt_blk: u32,
    pub pm1b_evt_blk: u32,
    pub pm1a_cnt_blk: u32,
    pub pm1b_cnt_blk: u32,

    pub pm2_cnt_blk: u32,
    pub pm_tmr_blk: u32,

    pub gpe0_blk: u32,
    pub gpe1_blk: u32,

    pub pm1_evt_len: u8,
    pub pm1_cnt_len: u8,
    pub pm2_cnt_len: u8,
    pub pm_tmr_len: u8,

    pub gpe0_blk_len: u8,
    pub gpe1_blk_len: u8,
    pub gpe1_base: u8,

    pub cst_cnt: u8,

    pub p_lvl2_lat: u16,
    pub p_lvl3_lat: u16,

    pub flush_size: u16,
    pub flush_stride: u16,

    pub duty_offset: u8,
    pub duty_width: u8,

    pub day_alrm: u8,
    pub mon_alrm: u8,

    pub century: u8,
    pub iapc_boot_arch: u16,
    pub reserved2: u8,
    pub flags: u32,
    pub reset_reg: GenericAddressStructure,
    pub reset_value: u8,
    pub arm_boot_arch: u16,
    pub fadt_minor_version: u8,

    pub x_firmware_ctrl: u64,
    pub x_dsdt: u64,

    pub x_pm1a_evt_blk: GenericAddressStructure,
    pub x_pm1b_evt_blk: GenericAddressStructure,
    pub x_pm1a_cnt_blk: GenericAddressStructure,
    pub x_pm1b_cnt_blk: GenericAddressStructure,
    pub x_pm2_cnt_blk: GenericAddressStructure,
    pub x_pm_tmr_blk: GenericAddressStructure,

    pub x_gpe0_blk: GenericAddressStructure,
    pub x_gpe1_blk: GenericAddressStructure,

    pub sleep_control_reg: GenericAddressStructure,
    pub sleep_status_reg: GenericAddressStructure,

    pub hypervisor_vendor_id: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct GenericAddressStructure {
    pub address_space: u8,
    pub bit_width: u8,
    pub bit_offset: u8,
    pub access_size: u8,
    pub address: u64,
}

#[repr(C, packed)]
#[derive(Debug)]
pub struct MADT {
    pub header: ACPIHeader,
    local_apic_address: u32,
    flags: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct MADTRecord {
    pub entry_type: u8,
    pub length: u8,
}

// any sdt
pub trait SDT: Send + Sync {
    fn header(&self) -> &ACPIHeader;

    fn len(&self) -> u32 {
        self.header().len
    }

    /// returns the address of element number n and it's size
    unsafe fn nth(&self, n: usize) -> (usize, usize);
}

// RSDT and RSDT
// stands for Parent Table of System Descriptors (yes it gave me ptsd)
pub trait PTSD: SDT + Send + Sync {
    unsafe fn get_entry(&self, signatrue: [u8; 4]) -> Option<*const ACPIHeader> {
        unsafe {
            for i in 0..(self.count()) {
                let item_ptr = self.nth(i).0 as *const ACPIHeader;
                let item = item_ptr.read_unaligned();

                let sign = item.signatrue;
                if sign == signatrue {
                    return Some(item_ptr);
                }
            }
            None
        }
    }

    // table item count
    fn count(&self) -> usize;
}

impl<'a> dyn PTSD + 'a {
    unsafe fn get_entry_cast<T: SDT>(&self, signatrue: [u8; 4]) -> Option<*const T> {
        unsafe { self.get_entry(signatrue).map(|p| p as *const T) }
    }
}

impl SDT for RSDT {
    fn header(&self) -> &ACPIHeader {
        &self.header
    }

    unsafe fn nth(&self, n: usize) -> (usize, usize) {
        unsafe {
            let addr = *self.table.as_ptr().add(n) as usize;
            let addr = addr | *HHDM;

            (addr, 0)
        }
    }
}

impl SDT for XSDT {
    fn header(&self) -> &ACPIHeader {
        &self.header
    }

    unsafe fn nth(&self, n: usize) -> (usize, usize) {
        unsafe {
            let table_ptr = addr_of!(self.table) as *const u64;
            let addr = table_ptr.add(n);
            let addr = core::ptr::read_unaligned(addr) as usize;
            let addr = addr | *HHDM;

            (addr, 0)
        }
    }
}

impl PTSD for XSDT {
    fn count(&self) -> usize {
        (self.len() as usize - size_of::<ACPIHeader>()) / size_of::<u64>()
    }
}
impl PTSD for RSDT {
    fn count(&self) -> usize {
        (self.len() as usize - size_of::<ACPIHeader>()) / size_of::<u32>()
    }
}

impl SDT for FADT {
    fn header(&self) -> &ACPIHeader {
        &self.header
    }

    unsafe fn nth(&self, _: usize) -> (usize, usize) {
        panic!("FADT SDT doesn't support nth!")
    }
}

impl FADT {
    fn get(ptsd: &dyn PTSD) -> &Self {
        unsafe { &*(ptsd.get_entry_cast(*b"FACP").unwrap()) }
    }
}

impl MCFG {
    pub fn nth(&self, n: usize) -> Option<MCFGEntry> {
        let table = addr_of!(self.entries) as *const MCFGEntry;
        unsafe {
            if n >= self.count() {
                None
            } else {
                let ptr = table.add(n);
                Some(core::ptr::read_unaligned(ptr))
            }
        }
    }

    fn get(ptsd: &dyn PTSD) -> &Self {
        unsafe { &*(ptsd.get_entry_cast(*b"MCFG").unwrap()) }
    }
    /// Returns the number of entries in [`Self`]
    pub fn count(&self) -> usize {
        let len = self.len() as usize;
        (len - size_of::<Self>()) / size_of::<MCFGEntry>()
    }
}

impl SDT for MCFG {
    fn header(&self) -> &ACPIHeader {
        &self.header
    }
    unsafe fn nth(&self, _: usize) -> (usize, usize) {
        unimplemented!()
    }
}

impl SDT for MADT {
    fn header(&self) -> &ACPIHeader {
        &self.header
    }

    unsafe fn nth(&self, n: usize) -> (usize, usize) {
        unsafe {
            let addr = self as *const Self;

            if n == 0 {
                let base = (addr).byte_add(size_of::<MADT>());
                return (base as usize, base as usize - addr as usize);
            }

            let base = self.nth(0).0;
            let mut record = base + (*(base as *const MADTRecord)).length as usize;

            for _ in 1..n - 1 {
                let next_record = record as *const MADTRecord;
                let len = (*next_record).length;
                record += len as usize;
            }

            (record, record - addr as usize)
        }
    }
}

impl MADT {
    pub unsafe fn get_record_of_type(&self, ty: u8) -> Option<*const MADTRecord> {
        unsafe {
            let len = self.header.len;
            let mut current_offset = 0;
            let mut i = 0;

            while current_offset <= len as usize {
                let (ptr, offset) = self.nth(i);
                let ptr = ptr as *const MADTRecord;

                if (*ptr).entry_type == ty {
                    return Some(ptr);
                }

                i += 1;
                current_offset = offset;
            }

            None
        }
    }

    pub fn get(ptsd: &dyn PTSD) -> &MADT {
        unsafe { &*(ptsd.get_entry_cast(*b"APIC").unwrap()) }
    }
}

fn get_rsdp() -> RSDPDesc {
    let addr = *RSDP_ADDR | *HHDM;
    let ptr = addr as *mut RSDPDesc;

    let desc = unsafe { *ptr };
    assert!(desc.vaildate());
    desc
}

fn get_sdt() -> &'static dyn PTSD {
    let rsdp = get_rsdp();

    if rsdp.xsdt_addr != 0 {
        let xsdt_addr = rsdp.xsdt_addr as usize | *HHDM;
        let xsdt_ptr = xsdt_addr as *const XSDT;

        return unsafe { &*xsdt_ptr };
    }

    let rsdt_addr = rsdp.rsdt_addr as usize | *HHDM;
    let rsdt_ptr = rsdt_addr as *const RSDT;

    unsafe { &*rsdt_ptr }
}
