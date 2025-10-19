use core::{mem::offset_of, ptr::addr_of};

use lazy_static::lazy_static;

use crate::{PhysAddr, RSDP_ADDR, VirtAddr};

lazy_static! {
    pub static ref PSDT_DESC: Option<GenericRootSDT> = get_sdt();
    pub static ref MADT_DESC: Option<&'static MADT> = MADT::get((*PSDT_DESC).as_ref()?);
    pub static ref MCFG_DESC: Option<&'static MCFG> = MCFG::get((*PSDT_DESC).as_ref()?);
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
    xsdt_addr: PhysAddr,
    extended_checksum: u8,
    reserved: [u8; 3],
}

const _: () = assert!(size_of::<RSDPDesc>() == 36);

impl RSDPDesc {
    pub fn vaildate(&self) -> bool {
        let size = if self.revision >= 2 { 36 } else { 20 };
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

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct RawRSDT {
    pub header: ACPIHeader,
    table: [u32; 0], // uint32_t table[];?
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct RawXSDT {
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
    type Element;
    fn header(&self) -> *const ACPIHeader;

    fn len(&self) -> u32 {
        unsafe { self.header().read_unaligned().len }
    }

    /// returns the address of element number n and it's size
    unsafe fn nth(&self, n: usize) -> (*const Self::Element, usize);
}

/// Generic wrapper around the root SDT eg. XSDT or RSDT
pub enum GenericRootSDT {
    RSDT(*const RawRSDT),
    XSDT(*const RawXSDT),
}

unsafe impl Send for GenericRootSDT {}
unsafe impl Sync for GenericRootSDT {}

impl SDT for GenericRootSDT {
    type Element = ACPIHeader;
    fn header(&self) -> *const ACPIHeader {
        match self {
            Self::RSDT(rsdt) => unsafe { rsdt.byte_add(offset_of!(RawRSDT, header)).cast() },
            Self::XSDT(xsdt) => unsafe { xsdt.byte_add(offset_of!(RawXSDT, header)).cast() },
        }
    }

    unsafe fn nth(&self, n: usize) -> (*const ACPIHeader, usize) {
        unsafe {
            let phys_addr = match self {
                Self::RSDT(rsdt) => PhysAddr::from(
                    (rsdt.byte_add(offset_of!(RawRSDT, table)) as *mut u32)
                        .add(n)
                        .read_unaligned() as usize,
                ),
                Self::XSDT(xsdt) => PhysAddr::from(
                    (xsdt.byte_add(offset_of!(RawXSDT, table)) as *mut u64)
                        .add(n)
                        .read_unaligned() as usize,
                ),
            };

            (phys_addr.into_virt().into_ptr(), 0)
        }
    }
}

impl GenericRootSDT {
    unsafe fn get_entry(&self, signatrue: [u8; 4]) -> Option<*const ACPIHeader> {
        unsafe {
            for i in 0..(self.count()) {
                let (item_ptr, _) = self.nth(i);
                let item = item_ptr.read_unaligned();

                let sign = item.signatrue;
                crate::serial!("sign is {sign:?}\n");
                if sign == signatrue {
                    return Some(item_ptr);
                }
            }
            None
        }
    }

    // table item count
    fn count(&self) -> usize {
        let unit_size = match self {
            Self::RSDT(_) => size_of::<u32>(),
            Self::XSDT(_) => size_of::<u64>(),
        };

        (self.len() as usize - size_of::<ACPIHeader>()) / unit_size
    }
    unsafe fn get_entry_cast_ref<T: SDT>(&self, signatrue: [u8; 4]) -> Option<&T> {
        unsafe { self.get_entry(signatrue).map(|p| &*(p as *const T)) }
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

    fn get(from: &GenericRootSDT) -> Option<&Self> {
        unsafe { from.get_entry_cast_ref(*b"MCFG") }
    }
    /// Returns the number of entries in [`Self`]
    pub fn count(&self) -> usize {
        let len = self.len() as usize;
        (len - size_of::<Self>()) / size_of::<MCFGEntry>()
    }
}

impl SDT for MCFG {
    type Element = ();
    fn header(&self) -> *const ACPIHeader {
        &self.header
    }
    unsafe fn nth(&self, _: usize) -> (*const (), usize) {
        unimplemented!()
    }
}

impl SDT for MADT {
    type Element = MADTRecord;
    fn header(&self) -> *const ACPIHeader {
        &self.header
    }

    unsafe fn nth(&self, n: usize) -> (*const MADTRecord, usize) {
        unsafe {
            if n == 0 {
                let self_base = VirtAddr::from_ptr(self as *const Self);
                let base = self_base + size_of::<MADT>();
                return (base.into_ptr(), base - self_base);
            }

            let (base_ptr, _) = self.nth(0);
            let mut record = base_ptr.byte_add(base_ptr.read_unaligned().length as usize);

            for _ in 1..n {
                let len = record.read_unaligned().length;
                record = record.byte_add(len as usize);
            }

            (record, record.byte_offset_from_unsigned(self))
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

    pub fn get(from: &GenericRootSDT) -> Option<&MADT> {
        unsafe { from.get_entry_cast_ref(*b"APIC") }
    }
}

fn get_rsdp() -> Option<RSDPDesc> {
    let addr = (*RSDP_ADDR)?.into_virt();
    let ptr = addr.into_ptr::<RSDPDesc>();

    let desc = unsafe { *ptr };
    assert!(desc.vaildate());
    Some(desc)
}

fn get_sdt() -> Option<GenericRootSDT> {
    let rsdp = get_rsdp()?;

    if rsdp.revision >= 2 {
        let xsdt_addr = rsdp.xsdt_addr;
        assert_ne!(xsdt_addr, PhysAddr::null());

        let xsdt_addr = xsdt_addr.into_virt();
        let xsdt_ptr = xsdt_addr.into_ptr::<RawXSDT>();

        Some(GenericRootSDT::XSDT(xsdt_ptr))
    } else {
        let rsdt_addr = PhysAddr::from(rsdp.rsdt_addr as usize).into_virt();
        let rsdt_ptr = rsdt_addr.into_ptr::<RawRSDT>();
        Some(GenericRootSDT::RSDT(rsdt_ptr))
    }
}
