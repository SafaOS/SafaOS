//! Process's Virtual Address Space related things

use crate::{
    VirtAddr,
    arch::paging::PageTable,
    drivers::vfs::FSObjectDescriptor,
    memory::{
        AlignToPage,
        frame_allocator::{self, Frame},
        paging::{self, MapToError, PAGE_SIZE, Page, PhysPageTable},
    },
};

/// A page aligned memory mapping that is unmapped (freed) when dropped
///
/// Can also repersent a memory mapped interface over a device or a file (the file or device is synced when dropped)
pub struct TrackedMemoryMapping {
    page_table: *mut PageTable,
    start_page: Page,
    end_page: Page,
    // on Drop syncs descriptor's last writes
    _obj_descriptor: Option<FSObjectDescriptor>,
}

impl TrackedMemoryMapping {
    pub const fn end(&self) -> VirtAddr {
        self.end_page.virt_addr()
    }
}

impl Drop for TrackedMemoryMapping {
    fn drop(&mut self) {
        unsafe {
            (*self.page_table).free_unmap(self.start_page.virt_addr(), self.end_page.virt_addr());
        }
    }
}

#[derive(Debug)]
/// Process Virtual Address Space Allocator
pub struct ProcVASA {
    pub(super) page_table: PhysPageTable,
    executable_end: VirtAddr,
    lookup_start: VirtAddr,

    data_break_pages: usize,
    data_break: VirtAddr,
}

impl ProcVASA {
    pub const fn new(page_table: PhysPageTable, executable_end: VirtAddr) -> Self {
        Self {
            page_table,
            executable_end,
            data_break: executable_end,
            // Gives sbrk 64 GiB of memory to use
            lookup_start: executable_end + 0x1000000000,
            data_break_pages: 0,
        }
    }
    /// Maps 'n' pages, taking `addr_hint` as a hint to where the mapping should start, using the flags `flags`,
    /// `guard_pages` pages will be kept unmapped after and before the mapping
    ///
    /// Will use the `frames_to_use` iterator as frames to map to until it returns None, in that case it will use newly allocated frames
    /// # Returns
    /// The first page and the last page in the paging, (the last page isn't really included it is an exclusive range)
    pub fn map_n_pages<I: Iterator<Item = Frame>>(
        &mut self,
        addr_hint: Option<VirtAddr>,
        n: usize,
        guard_pages: usize,
        flags: paging::EntryFlags,
        mut frames_to_use: I,
    ) -> Result<(Page, Page), MapToError> {
        if n == 0 {
            return Ok((
                Page::containing_address(VirtAddr::null()),
                Page::containing_address(VirtAddr::null()),
            ));
        }

        let bytes_wanted = n * PAGE_SIZE;
        let guard_bytes = guard_pages * PAGE_SIZE;

        let lookup_start = addr_hint.unwrap_or(self.lookup_start);
        let mut looking_at = Page::containing_address(lookup_start);

        let (map_start, map_end) = loop {
            while self.page_table.get_frame(looking_at).is_some() {
                looking_at = looking_at.next();
            }

            let map_start = Page::containing_address(VirtAddr::from(
                looking_at.virt_addr().saturating_sub(guard_bytes),
            ));
            let map_end =
                Page::containing_address(looking_at.virt_addr() + bytes_wanted + guard_bytes);

            for page in Page::iter_pages(map_start, map_end) {
                if self.page_table.get_frame(page).is_some() {
                    continue;
                }
            }

            let actual_map_end = Page::containing_address(map_end.virt_addr() - guard_bytes);
            break (looking_at, actual_map_end);
        };

        let pages = Page::iter_pages(map_start, map_end);
        for page in pages {
            let frame = frames_to_use
                .next()
                .or_else(|| frame_allocator::allocate_frame())
                .ok_or(MapToError::FrameAllocationFailed)?;

            unsafe {
                assert_ne!(
                    self.page_table.map_zeroed_to_uncached(page, frame, flags),
                    Err(MapToError::AlreadyMapped)
                );
            }
        }

        self.page_table.flush_cache();

        self.lookup_start = map_end.virt_addr() + guard_bytes;
        Ok((map_start, map_end))
    }

    /// Maps 'n' pages, taking `addr_hint` as a hint to where the mapping should start, using the flags `flags`
    ///
    /// Will use the `frames_to_use` iterator as frames to map to until it returns None, in that case it will use newly allocated frames
    /// Returns a Tracker that frees the mapping on Drop
    /// # Returns
    /// The start address of the mapping
    pub fn map_n_pages_tracked<I: Iterator<Item = Frame>>(
        &mut self,
        addr_hint: Option<VirtAddr>,
        n: usize,
        guard_pages: usize,
        flags: paging::EntryFlags,
        frames_to_use: I,
        tracked_fs_obj: Option<FSObjectDescriptor>,
    ) -> Result<TrackedMemoryMapping, MapToError> {
        let (start_page, end_page) =
            self.map_n_pages(addr_hint, n, guard_pages, flags, frames_to_use)?;
        Ok(TrackedMemoryMapping {
            page_table: &mut *self.page_table,
            start_page,
            end_page,
            _obj_descriptor: tracked_fs_obj,
        })
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
}
