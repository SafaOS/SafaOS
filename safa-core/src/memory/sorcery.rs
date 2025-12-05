use super::{
    VirtAddr,
    frame_allocator::FramePtr,
    paging::{EntryFlags, PAGE_SIZE},
};
use ::limine::memory_map::EntryType;

use crate::{
    PhysAddr,
    arch::{self, paging::set_current_higher_page_table},
    debug,
    limine::{self, executable_phys_address, executable_virt_address},
    memory::{AlignToPage, HHDM, frame_allocator},
};

use super::paging::{MapToError, PageTable};

pub const HEAP: (VirtAddr, VirtAddr) = {
    // assuming HHDM starts at 0xffff000000000000
    // this allows for 224 TiBs of HHDM
    // assuming it starts at 0xffff800000000000
    // this allows for 96 TiBs of HHDM meaning you don't really have to worry`
    let end = VirtAddr::from(0xffffe00000000000);
    // 2 TiB from end
    (end, end + (0x100000000000 / 8))
};

pub const LARGE_HEAP: (VirtAddr, VirtAddr) = {
    let (_, end) = HEAP;
    // 4 TiB from end
    (end, end + (0x100000000000 / 4))
};

fn create_root_page_table() -> Result<FramePtr<PageTable>, MapToError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

    let mut table = unsafe { frame.into_ptr::<PageTable>() };
    table.zeroize();
    unsafe {
        let dest = &mut *table;

        map_hhdm(dest)?;
        arch::paging::map_devices(dest)?;
        map_top_2gb(dest)?;
    }

    Ok(table)
}

unsafe fn map_hhdm(dest: &mut PageTable) -> Result<VirtAddr, MapToError> {
    debug!(
        PageTable,
        "mapping HHDM, limine's: {:#x}",
        limine::get_phy_offset()
    );

    let flags = EntryFlags::WRITE;
    for entry in limine::mmap_request().entries() {
        let phys_addr = PhysAddr::from(entry.base as usize);
        let size_bytes = entry.length as usize;
        let size = size_bytes.to_next_page();

        if entry.entry_type != EntryType::BAD_MEMORY && entry.entry_type != EntryType::RESERVED {
            let flags = if entry.entry_type == EntryType::FRAMEBUFFER {
                flags | EntryFlags::FRAMEBUFFER_CACHED
            } else if entry.entry_type == EntryType::USABLE {
                // Normal memory == normal caching
                flags
            } else {
                flags | EntryFlags::DEVICE_UNCACHEABLE
            };

            let virt_addr = phys_addr.into_virt();
            let page_num = size / PAGE_SIZE;

            unsafe {
                dest.map_contiguous_pages(virt_addr, phys_addr, page_num, flags)?;
            }
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

unsafe fn map_top_2gb(dest: &mut PageTable) -> Result<(), MapToError> {
    unsafe {
        debug!(PageTable, "mapping kernel");
        let virt_addr = executable_virt_address();
        let phys_addr = executable_phys_address();

        let mut map_section = |name: &'static str,
                               section_virt_begin: VirtAddr,
                               section_virt_end: VirtAddr,
                               flags: EntryFlags| {
            let section_off = section_virt_begin - virt_addr;
            let section_phys_begin = phys_addr + section_off;
            let section_size = section_virt_end - section_virt_begin;
            debug!(
                PageTable,
                "Mapping {name}: {section_virt_begin:?}..{section_virt_end:?} => {section_phys_begin:?}..{:?} ({section_size}bytes) with flags {flags:?}",
                section_phys_begin + section_size
            );

            dest.map_contiguous_pages(
                section_virt_begin,
                section_phys_begin,
                section_size.div_ceil(PAGE_SIZE),
                flags,
            )?;

            debug!(PageTable, "Mapped {name}");
            Ok(())
        };

        map_section(
            ".text",
            VirtAddr::from_ptr(&section_text_begin),
            VirtAddr::from_ptr(&section_text_end),
            EntryFlags::empty(),
        )?;
        map_section(
            ".rodata",
            VirtAddr::from_ptr(&section_rodata_begin),
            VirtAddr::from_ptr(&section_rodata_end),
            EntryFlags::DISABLE_EXEC,
        )?;
        map_section(
            ".data",
            VirtAddr::from_ptr(&section_data_begin),
            VirtAddr::from_ptr(&section_data_end),
            EntryFlags::WRITE | EntryFlags::DISABLE_EXEC,
        )?;

        debug!(PageTable, "mapped kernel");
        Ok(())
    }
}

pub fn init_page_table() {
    debug!(PageTable, "initializing root page table ... ");
    let _ = unsafe { super::paging::current_higher_root_table() };
    let table = create_root_page_table().unwrap();
    unsafe {
        set_current_higher_page_table(table);
    }
    // de-allocating the previous root table
    // FIXME: could still be used by other cpus so i don't free it for now
    // let frame = previous_table.frame();
    // frame_allocator::deallocate_frame(frame)
}
