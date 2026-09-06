use crate::{
    arch,
    bootloader::{self, HHDM, MemoryType},
    logging,
    memory::{
        phys_to_virt,
        vmm::{Location, VMMAllocError, VMMMFlags, VirtualMemoryManager},
    },
    misc::PAGE_SIZE,
    paging::OwnedPageTable,
    percpu,
};

use super::VirtAddr;

fn create_vmm() -> Result<VirtualMemoryManager, VMMAllocError> {
    let table = OwnedPageTable::create()?;

    let mut vmm = VirtualMemoryManager::new(
        *HHDM,
        VirtAddr::from(usize::MAX) - *HHDM,
        table.into_table(),
    );

    unsafe {
        let hhdm_end = map_hhdm(&mut vmm)?;
        logging::trace!(PageTable, "HHDM ends at: {hhdm_end:?}");
        map_top_2gb(&mut vmm)?;
    }

    Ok(vmm)
}

unsafe fn map_hhdm(dest: &mut VirtualMemoryManager) -> Result<VirtAddr, VMMAllocError> {
    logging::debug!(PageTable, "mapping HHDM at {:?}", *HHDM);
    assert!(!HHDM.is_in_lower_half());

    let flags = VMMMFlags::WRITABLE | VMMMFlags::_GLOBAL_HINT;

    let mut largest_addr = VirtAddr::null();
    for entry in bootloader::memory_map() {
        let phys_addr = entry.base;
        let size_bytes = entry.size as usize;
        let size = size_bytes.next_multiple_of(PAGE_SIZE);

        if entry.kind != MemoryType::Bad && entry.kind != MemoryType::Reserved {
            let (flags, name) = if entry.kind == MemoryType::Framebuffer {
                (flags | VMMMFlags::FRAMEBUFFER_CACHED, &"FRAMEBUFFER")
            } else {
                (flags, &"HHDM")
            };

            let virt_addr = phys_to_virt(phys_addr);
            let page_num = size / PAGE_SIZE;

            dest.map_direct_phys(
                name,
                Some(Location::Fixed(virt_addr)),
                phys_addr,
                page_num,
                flags,
            )?;

            largest_addr = largest_addr.max(virt_addr + size);
        }
    }

    logging::info!(
        PageTable,
        "mapped HHDM from {:?} to {:?}",
        *HHDM,
        largest_addr
    );
    Ok(largest_addr + PAGE_SIZE)
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
        logging::debug!(PageTable, "mapping kernel");

        let virt_addr = bootloader::exe_virt();
        let phys_addr = bootloader::exe_phys();

        let map_section = |name: &'static str,
                           section_virt_begin: VirtAddr,
                           section_virt_end: VirtAddr,
                           flags: VMMMFlags| {
            let section_off = section_virt_begin - virt_addr;
            let section_phys_begin = phys_addr + section_off;
            let section_size = section_virt_end - section_virt_begin;
            logging::debug!(
                PageTable,
                "Mapping {name}: {section_virt_begin:?}..{section_virt_end:?} => {section_phys_begin:?}..{:?} ({section_size}bytes) with flags {flags:?}",
                section_phys_begin + section_size
            );

            vmm.map_direct_phys(
                &"KERNEL",
                Some(Location::Fixed(section_virt_begin)),
                section_phys_begin,
                section_size.div_ceil(PAGE_SIZE),
                flags | VMMMFlags::_GLOBAL_HINT,
            )?;

            logging::debug!(PageTable, "Mapped {name}");
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
            VMMMFlags::WRITABLE,
        )?;

        percpu::init_memory(vmm);
        logging::debug!(PageTable, "mapped kernel");
        Ok(())
    }
}

/// Inits the page table and the VMM
pub fn init_all() {
    logging::debug!(PageTable, "initializing root page table ... ");
    let vmm = create_vmm().expect("Failed to create root VMM");
    let addr = vmm.table_addr();

    unsafe {
        arch::paging::set_kernel_current(addr);
        super::vmm::init(vmm);
    }
}
