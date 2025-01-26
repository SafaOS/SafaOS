use super::paging::PAGE_SIZE;
use core::{arch::asm, fmt::Display};
use lazy_static::lazy_static;

use crate::{
    debug,
    limine::{self, MEMORY_END},
    memory::frame_allocator::{self, Frame},
};

use super::paging::{MapToError, Page, PageTable};

pub struct PageTableBinding {
    name: &'static str,
    from: (usize, usize),
    to: (usize, usize),
}

pub struct PageTableBindings<const N: usize> {
    bindings: [PageTableBinding; N],
}

impl PageTableBinding {
    pub fn apply_binding(&self, from_page_table: &mut PageTable, to_page_table: &mut PageTable) {
        let from_iter = Page::iter_pages(
            Page::containing_address(self.from.0),
            Page::containing_address(self.from.1),
        );

        let mut to_iter = Page::iter_pages(
            Page::containing_address(self.to.0),
            Page::containing_address(self.to.1),
        );

        for page in from_iter {
            let pml4_index = super::translate(page.start_address).3;
            let to_page = to_iter.next().unwrap();
            let to_plm4_index = super::translate(to_page.start_address).3;

            to_page_table[to_plm4_index] = from_page_table[pml4_index].clone();
        }
    }
}

impl Display for PageTableBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}: {:#x} .. {:#x} <{:#x} .. {:#x}>",
            self.name, self.to.0, self.to.1, self.from.0, self.from.1
        )
    }
}

impl<const N: usize> PageTableBindings<N> {
    pub fn apply_bindings(&self, from_page_table: &mut PageTable, to_page_table: &mut PageTable) {
        debug!(PageTableBindings<N>, "applying:\n{}", self);

        for binding in &self.bindings {
            binding.apply_binding(from_page_table, to_page_table);
        }
        debug!(PageTableBindings<N>, "done");
    }

    /// gets a PageTableBinding named `name`, returns it's start address as a pointer and it's
    /// size, pointer is vaild only if that binding is applied on the current page table, and it's
    /// pages is mapped
    pub fn get(&self, name: &'static str) -> Option<(*mut u8, usize)> {
        for binding in &self.bindings {
            if binding.name == name {
                return Some((binding.to.0 as *mut u8, binding.to.1 - binding.to.0));
            }
        }

        None
    }
    /// creates a page table from bindings applied from current root pagetable
    pub fn create_page_table(&self) -> Result<&'static mut PageTable, MapToError> {
        let table = {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

            let virt_start_addr = frame.virt_addr();
            let table = unsafe { &mut *(virt_start_addr as *mut PageTable) };

            table.zeroize();
            table
        };

        unsafe {
            self.apply_bindings(super::current_root_table(), table);
        }
        Ok(table)
    }
}

impl<const N: usize> Display for PageTableBindings<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for binding in &self.bindings {
            writeln!(f, "{}", binding)?;
        }
        Ok(())
    }
}

macro_rules! create_page_table_bindings {
    ($($name: literal => {$from_start: expr, $from_end: expr $(=> $to_start: expr, $to_end: expr)?}),*) => {
           PageTableBindings{bindings: [$(
                            PageTableBinding {
                                name: $name,
                                from: ($from_start, $from_end),
                                to: { ($from_start, $from_end) $(; ($to_start, $to_end))? },
                            },
            )*]}
    };
}

lazy_static! {
    pub static ref ROOT_BINDINGS: PageTableBindings<4> = {
        let heap_start = limine::get_phy_offset_end();
        let heap_end = heap_start + *MEMORY_END;
        // we only want to keep the phys mem mapping and the TOP_MOST_2GB
        // assuming that the framebuffer and all the kernel modules is in the range of phys_mem
        // getting page faults is better then UB
        create_page_table_bindings!(
            "PHYS_MEM" => { limine::get_phy_offset(), limine::get_phy_offset_end() },
            "HEAP" => { 0, 0 => heap_start, heap_end },
            "LARGE_HEAP" => { 0, 0 => super::align_up(heap_end, PAGE_SIZE), 0xffffffff80000000 },
            "TOP_MOST_2GB" => { 0xffffffff80000000, 0xffffffffffffffff }
        )
    };
}
pub fn create_root_page_table() -> Result<&'static mut PageTable, MapToError> {
    ROOT_BINDINGS.create_page_table()
}

/// sets the current Page Table to `page_table`
pub fn set_current_page_table(page_table: &'static mut PageTable) {
    let phys_addr = page_table as *mut _ as usize - limine::get_phy_offset();
    unsafe {
        asm!("mov cr3, rax", in("rax") phys_addr);
    }
}

pub fn init_page_table() {
    debug!(PageTable, "intializing root page table ... ");
    let previous_table = unsafe { super::current_root_table() };
    let table = create_root_page_table().unwrap();
    set_current_page_table(table);
    // de-allocating the previous root table
    let virt_addr = previous_table as *mut _ as usize;
    let frame = Frame::containing_address(virt_addr - limine::get_phy_offset());
    frame_allocator::deallocate_frame(frame)
}
