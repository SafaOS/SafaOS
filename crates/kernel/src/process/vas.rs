//! Process's Virtual Address Space related things

use core::mem::ManuallyDrop;

use crate::{
    VirtAddr,
    memory::{
        AlignToPage,
        frame_allocator::{self, Frame},
        paging::{self, MapToError, PAGE_SIZE, Page, PhysPageTable},
    },
};

#[derive(Debug)]
/// Process Virtual Address Space Allocator
pub struct ProcVASA {
    pub(super) page_table: ManuallyDrop<PhysPageTable>,
    executable_end: VirtAddr,

    data_break_pages: usize,
    data_break: VirtAddr,
}

impl ProcVASA {
    pub const fn new(page_table: PhysPageTable, executable_end: VirtAddr) -> Self {
        Self {
            page_table: ManuallyDrop::new(page_table),
            executable_end,
            data_break: executable_end,
            data_break_pages: 0,
        }
    }
    /// Maps 'n' pages, taking `addr_hint` as a hint to where the mapping should start, using the flags `flags`
    ///
    /// Will use the `frames_to_use` iterator as frames to map to until it returns None, in that case it will use newly allocated frames
    pub fn map_n_pages<I: Iterator<Item = Frame>>(
        &mut self,
        addr_hint: Option<VirtAddr>,
        n: usize,
        flags: paging::EntryFlags,
        mut frames_to_use: I,
    ) -> Result<VirtAddr, MapToError> {
        let bytes_wanted = n * PAGE_SIZE;

        let lookup_start = addr_hint.unwrap_or(self.executable_end);
        let mut looking_at = Page::containing_address(lookup_start);

        let (map_start, map_end) = loop {
            while self.page_table.get_frame(looking_at).is_some() {
                looking_at = looking_at.next();
            }

            let map_end = Page::containing_address(looking_at.virt_addr() + bytes_wanted);
            for page in Page::iter_pages(looking_at, map_end) {
                if self.page_table.get_frame(page).is_some() {
                    continue;
                }
            }

            break (looking_at, map_end);
        };

        let pages = Page::iter_pages(map_start, map_end);
        for page in pages {
            let frame = frames_to_use
                .next()
                .or_else(|| frame_allocator::allocate_frame())
                .ok_or(MapToError::FrameAllocationFailed)?;

            unsafe {
                assert_ne!(
                    self.page_table.map_zeroed_to(page, frame, flags),
                    Err(MapToError::AlreadyMapped)
                );
            }
        }

        Ok(map_start.virt_addr())
    }

    // !!!!! Code below is Deprecated but is still there for compatiblitiy purposes !!!!!
    fn page_extend_data(&mut self) -> Result<VirtAddr, MapToError> {
        use crate::memory::paging::EntryFlags;

        let page_end = self.executable_end + PAGE_SIZE * self.data_break_pages;
        let new_page = Page::containing_address(page_end);

        unsafe {
            self.page_table
                .map_zeroed(new_page, EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE)?;
        }

        self.data_break_pages += 1;
        Ok(new_page.virt_addr())
    }

    fn page_unextend_data(&mut self) -> VirtAddr {
        if self.data_break_pages == 0 {
            return self.executable_end;
        }

        let page_end = self.executable_end + PAGE_SIZE * self.data_break_pages;
        let page_addr = page_end - PAGE_SIZE;
        let page = Page::containing_address(page_addr);

        unsafe {
            self.page_table.unmap(page);
        }

        self.data_break_pages -= 1;
        page_addr
    }

    /// Maps `amount` more bytes of memory beyond the executable's end
    pub fn extend_data_by(&mut self, amount: isize) -> Result<*mut u8, MapToError> {
        let actual_data_break = self.executable_end + PAGE_SIZE * self.data_break_pages;
        let usable_bytes = actual_data_break - self.data_break;
        let is_negative = amount.is_negative();
        let amount = amount.unsigned_abs();

        if (usable_bytes < amount) || (is_negative) {
            let pages = (amount - usable_bytes).to_next_page() / PAGE_SIZE;

            for _ in 0..pages {
                // FIXME: not tested (is_negative)
                if is_negative {
                    self.page_unextend_data();
                } else {
                    self.page_extend_data()?;
                }
            }
        }

        if is_negative && amount >= usable_bytes {
            self.data_break -= amount;
        } else {
            self.data_break += amount;
        }

        Ok(self.data_break.into_ptr::<u8>())
    }

    /// Takes the page table self uses
    /// the page table can then be dropped and self becomes invalid
    unsafe fn take_pagetable(&mut self) -> PhysPageTable {
        unsafe { ManuallyDrop::take(&mut self.page_table) }
    }
}
