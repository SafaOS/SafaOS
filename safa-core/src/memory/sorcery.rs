use super::{VirtAddr, paging::PAGE_SIZE};
use ::limine::memory_map::EntryType;

use crate::{
    PhysAddr,
    arch::{self, paging::set_current_higher_page_table},
    debug,
    limine::{self, executable_phys_address, executable_virt_address},
    memory::{
        AlignToPage, HHDM, frame_allocator,
        vmm::{VMMAllocError, VMMMFlags, VirtualMemoryManager},
    },
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

fn create_vmm() -> Result<VirtualMemoryManager, VMMAllocError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

    let mut table = unsafe { frame.into_ptr::<PageTable>() };
    table.zeroize();
    let mut vmm = VirtualMemoryManager::new(HHDM, VirtAddr::from(usize::MAX) - HHDM, table);

    unsafe {
        map_hhdm(&mut vmm)?;
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
            } else if entry.entry_type == EntryType::USABLE {
                // Normal memory == normal caching
                (flags, &"HHDM")
            } else {
                (flags | VMMMFlags::UNCACHABLE, &"HHDM")
            };

            let virt_addr = phys_addr.into_virt();
            let page_num = size / PAGE_SIZE;

            dest.map_direct_phys(name, Some(virt_addr), phys_addr, page_num, flags)?;
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

        let mut map_section = |name: &'static str,
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
                Some(section_virt_begin),
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

        debug!(PageTable, "mapped kernel");
        Ok(())
    }
}

/// Inits the page table and the VMM
pub fn init_page_table() {
    debug!(PageTable, "initializing root page table ... ");
    let _ = unsafe { super::paging::current_higher_root_table() };
    let vmm = create_vmm().expect("Failed to create root VMM");
    unsafe {
        set_current_higher_page_table(vmm.table_ptr());
    }

    super::vmm::init(vmm);
    // de-allocating the previous root table
    // FIXME: could still be used by other cpus so i don't free it for now
    // let frame = previous_table.frame();
    // frame_allocator::deallocate_frame(frame)
}
