use core::cell::SyncUnsafeCell;

use super::{VirtAddr, paging::PAGE_SIZE};
use ::limine::memory_map::EntryType;

use crate::{
    PhysAddr,
    arch::{self, paging::set_current_higher_page_table},
    debug,
    limine::{self, executable_phys_address, executable_virt_address},
    memory::{
        AlignToPage, HHDM, frame_allocator,
        vmm::{Location, VMMAllocError, VMMMFlags, VirtualMemoryManager},
    },
    percpu,
};

use super::paging::{MapToError, PageTable};

static HEAP0_HINT: SyncUnsafeCell<VirtAddr> =
    SyncUnsafeCell::new(VirtAddr::from(0xffffe00000000000));

/// A hint to avoid fragmentation for a large free area for a heap
pub fn heap0_hint() -> VirtAddr {
    unsafe { *HEAP0_HINT.get() }
}

fn create_vmm() -> Result<VirtualMemoryManager, VMMAllocError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

    let mut table = unsafe { frame.into_ptr::<PageTable>() };
    table.zeroize();
    let mut vmm = VirtualMemoryManager::new(HHDM, VirtAddr::from(usize::MAX) - HHDM, table);

    unsafe {
        let hhdm_end = map_hhdm(&mut vmm)?;
        // 1TiB after HHDM end.
        *HEAP0_HINT.get() = hhdm_end + (1024 * 1024 * 1024 * 1024);
        map_top_2gb(&mut vmm)?;

        arch::paging::map_devices(&mut vmm)?;
    }

    Ok(vmm)
}

unsafe fn map_hhdm(dest: &mut VirtualMemoryManager) -> Result<VirtAddr, VMMAllocError> {
    debug!(
        PageTable,
        "mapping HHDM, limine's: {:#x}",
        limine::get_phy_offset()
    );

    let flags = VMMMFlags::WRITEABLE;
    for entry in limine::mmap_request().entries() {
        let phys_addr = PhysAddr::from(entry.base as usize);
        let size_bytes = entry.length as usize;
        let size = size_bytes.to_next_page();

        if entry.entry_type != EntryType::BAD_MEMORY && entry.entry_type != EntryType::RESERVED {
            let (flags, name) = if entry.entry_type == EntryType::FRAMEBUFFER {
                (flags | VMMMFlags::FRAMEBUFFER_CACHED, &"FRAMEBUFFER")
            } else {
                (flags | VMMMFlags::UNCACHABLE, &"HHDM")
            };

            let virt_addr = phys_addr.into_virt();
            let page_num = size / PAGE_SIZE;

            dest.map_direct_phys(
                name,
                Some(Location::Fixed(virt_addr)),
                phys_addr,
                page_num,
                flags,
            )?;
        }
    }

    // last possible virtual HHDM address
    // FIXME: hardcoded because if I rely on the memory map there are still some stuff out of the range of the last entry

    let largest_addr_virt = PhysAddr::from(0x10000000000).into_virt();
    debug!(
        PageTable,
        "mapped HHDM from {:#x} to {:?}", HHDM, largest_addr_virt
    );
    Ok(largest_addr_virt + PAGE_SIZE)
}

unsafe extern "C" {
    static section_text_begin: u8;
    static section_data_begin: u8;
    static section_rodata_begin: u8;
    static section_text_end: u8;
    static section_data_end: u8;
    static section_rodata_end: u8;
}

unsafe fn map_top_2gb(vmm: &mut VirtualMemoryManager) -> Result<(), VMMAllocError> {
    unsafe {
        debug!(PageTable, "mapping kernel");

        let virt_addr = executable_virt_address();
        let phys_addr = executable_phys_address();

        let map_section = |name: &'static str,
                           section_virt_begin: VirtAddr,
                           section_virt_end: VirtAddr,
                           flags: VMMMFlags| {
            let section_off = section_virt_begin - virt_addr;
            let section_phys_begin = phys_addr + section_off;
            let section_size = section_virt_end - section_virt_begin;
            debug!(
                PageTable,
                "Mapping {name}: {section_virt_begin:?}..{section_virt_end:?} => {section_phys_begin:?}..{:?} ({section_size}bytes) with flags {flags:?}",
                section_phys_begin + section_size
            );

            vmm.map_direct_phys(
                &"KERNEL",
                Some(Location::Fixed(section_virt_begin)),
                section_phys_begin,
                section_size.div_ceil(PAGE_SIZE),
                flags,
            )?;

            debug!(PageTable, "Mapped {name}");
            Ok::<_, VMMAllocError>(())
        };

        map_section(
            ".text",
            VirtAddr::from_ptr(&section_text_begin),
            VirtAddr::from_ptr(&section_text_end),
            VMMMFlags::EXECUTABLE,
        )?;
        map_section(
            ".rodata",
            VirtAddr::from_ptr(&section_rodata_begin),
            VirtAddr::from_ptr(&section_rodata_end),
            VMMMFlags::empty(),
        )?;
        map_section(
            ".data",
            VirtAddr::from_ptr(&section_data_begin),
            VirtAddr::from_ptr(&section_data_end),
            VMMMFlags::WRITEABLE,
        )?;

        percpu::init_memory(vmm);
        debug!(PageTable, "mapped kernel");
        Ok(())
    }
}

/// Inits the page table and the VMM
pub fn init_all() {
    debug!(PageTable, "initializing root page table ... ");
    let vmm = create_vmm().expect("Failed to create root VMM");
    let table = unsafe { vmm.table_ptr() };

    unsafe {
        set_current_higher_page_table(table);
        super::vmm::init(vmm);
    }
}
