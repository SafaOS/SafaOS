#![allow(unreachable_code)]
#![allow(unused_variables)]
use crate::{
    memory::{
        frame_allocator::{Frame, FramePtr},
        paging::{EntryFlags, MapToError, Page},
    },
    VirtAddr,
};
use core::ops::{Index, IndexMut};

/// A hack that returns the last level (root) table's index from a VirtAddr
/// level 4 in x86_64
/// l0 in aarch64
/// FIXME: bad usage
pub const fn root_table_index(addr: VirtAddr) -> usize {
    todo!()
}

#[derive(Clone)]
/// A page table's entry
pub struct Entry(!);

#[derive(Debug, Clone)]
#[repr(C)]
pub struct PageTable(!);

impl Index<usize> for PageTable {
    type Output = Entry;
    fn index(&self, index: usize) -> &Self::Output {
        self.0
    }
}

impl IndexMut<usize> for PageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.0
    }
}

/// returns the current higher half root page table
/// in x86_64 the higher half table is the same as the lower one which is not the case in aarch64
pub unsafe fn current_higher_root_table() -> FramePtr<PageTable> {
    todo!()
}

/// returns the current lower half root page table
/// in x86_64 the higher half table is the same as the lower one which is not the case in aarch64
pub unsafe fn current_lower_root_table() -> FramePtr<PageTable> {
    todo!()
}

/// sets the current higher half Page Table to `page_table`
pub unsafe fn set_current_higher_page_table(page_table: FramePtr<PageTable>) {
    todo!()
}

impl PageTable {
    pub fn zeroize(&mut self) {
        self.0
    }

    /// copies the higher half entries of the current pml4 to this page table
    pub fn copy_higher_half(&mut self) {
        self.0
    }
    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    pub unsafe fn free(&mut self, level: u8) {
        self.0
    }

    /// maps a virtual `Page` to physical `Frame`
    pub fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        self.0
    }

    /// gets the frame page points to
    pub fn get_frame(&mut self, page: Page) -> Option<Frame> {
        self.0
    }

    /// unmap page and all of it's entries
    pub fn unmap(&mut self, page: Page) {
        self.0
    }
}
