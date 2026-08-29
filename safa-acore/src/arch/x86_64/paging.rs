use bitflags::bitflags;
use core::cell::SyncUnsafeCell;
use core::fmt::Debug;
use core::ops::IndexMut;
use core::ptr::NonNull;
use core::{arch::asm, ops::Index};

use crate::memory::{frame_allocator, phys_to_virt};
use crate::misc::{FrameIter, IterPage, PAGE_SIZE, VirtAddr};
use crate::paging::{PageEntryFlags, PageTableOps};
use crate::{
    misc::{Frame, Page, PhysAddr},
    paging::MapToError,
};
use bitfield_struct::bitfield;

const ENTRY_COUNT: usize = 512;
const HIGHER_HALF_ENTRY: usize = 256;

const fn p4_index(addr: usize) -> usize {
    (addr >> 39) & 0x1FF
}
const fn p3_index(addr: usize) -> usize {
    (addr >> 30) & 0x1FF
}
const fn p2_index(addr: usize) -> usize {
    (addr >> 21) & 0x1FF
}
const fn p1_index(addr: usize) -> usize {
    (addr >> 12) & 0x1FF
}

const fn translate(addr: VirtAddr) -> (usize, usize, usize, usize) {
    let addr = addr.raw();
    (
        p1_index(addr),
        p2_index(addr),
        p3_index(addr),
        p4_index(addr),
    )
}

#[bitfield(u64)]
/// A page table's entry
pub struct Entry {
    /// The entry is valid.
    present: bool,
    /// The memory presented is read/write.
    writable: bool,
    /// The memory is accessible by ring0.
    user_accessible: bool,
    /// PWT Writing to this page would directly write to both memory and the cache, not just the cache.
    write_through: bool,
    /// PCD no caching for this page's reads/writes.
    cache_disable: bool,
    /// Hint bit, when the CPU access this entry it would be set.
    accessed_hint: bool,
    /// when a write happens to this Page, the CPU should set this bit to true.
    dirty_hint: bool,
    /// When this bit is set the page is either a Huge Page in case it isn't a level 4 entry, or this alongside [`Self::cache_disable`] and [`Self::write_through`] shall be used.
    pat_or_hugepage: bool,
    /// When this bit is set the page is global and therefore it's TLB won't be discarded when cr3 is reloaded.
    /// Useful for performance with higher-half kernel sharing.
    global: bool,
    /// Free available bits for later software use.
    #[bits(3)]
    avl: u8,
    /// The number of the physical memory page that this entry points to == physical address >> 12
    #[bits(39)]
    pagenum: usize,
    __rsvd0: bool,
    /// Free available bits for later software use.
    #[bits(7)]
    avl1: u8,
    /// Protection Key.
    #[bits(4)]
    pk: u8,
    execute_disable: bool,
}

/// Index of write combining in the x86_64 PAT.
///
/// This is the index limine provides.
pub const PAT_WC_INDEX: u8 = 5;

impl Entry {
    const fn set_pat_index(&mut self, index: u8) {
        assert!(index <= 0b111, "PAT index out of range");

        // The PAT is indexed by the three page table bits:
        // PAT | PCD | PWT
        let pwt = (index & 0b001) != 0;
        let pcd = (index & 0b010) != 0;
        let pat = (index & 0b100) != 0;

        // Limine also sets up write through as index 1, and uncachable as index 2, so there is no collision.
        self.set_write_through(pwt);
        self.set_cache_disable(pcd);
        self.set_pat_or_hugepage(pat);
    }

    /// Returns a new present entry constructed from given flags, without an address.
    const fn new_from_flags(value: PageEntryFlags, is_level4: bool) -> Self {
        let mut this = Self::new().with_present(true);

        if value.contains(PageEntryFlags::WRITE) {
            this.set_writable(true);
        }

        if value.contains(PageEntryFlags::DEVICE_UNCACHEABLE) {
            this.set_cache_disable(true);
        }

        // We cannot use the pat bit if it isn't level 4
        if value.contains(PageEntryFlags::FRAMEBUFFER_CACHED) && is_level4 {
            this.set_pat_index(PAT_WC_INDEX);
        }

        if value.contains(PageEntryFlags::USER_ACCESSIBLE) {
            this.set_user_accessible(true);
        }

        if value.contains(PageEntryFlags::DISABLE_EXEC) {
            this.set_execute_disable(true);
        }

        if value.contains(PageEntryFlags::_GLOBAL_SHARED) {
            this.set_global(true);
        }

        this
    }
    /// Returns the physical address associated with this entry if it is present.
    #[inline(always)]
    fn addr(&self) -> Option<PhysAddr> {
        self.present()
            .then(|| PhysAddr::new(self.pagenum() * PAGE_SIZE))
    }

    #[inline(always)]
    fn frame(&self) -> Option<Frame> {
        self.addr().map(|a| Frame::containing(a))
    }

    /// Sets the physical address this entry points to.
    #[inline(always)]
    fn set_addr(&mut self, addr: PhysAddr) {
        self.set_pagenum(addr.page_num());
    }

    #[inline(always)]
    fn with_addr(mut self, addr: PhysAddr) -> Self {
        self.set_addr(addr);
        self
    }

    #[inline(always)]
    fn zeroize(&mut self) {
        self.0 = 0;
    }

    /// deallocates an entry depending on it's level if it is 1 it should just deallocate the frame
    /// otherwise treat the frame as a page table and deallocate it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    unsafe fn free(&mut self, level: u8) {
        unsafe {
            if level != 0 {
                let addr = self
                    .addr()
                    .expect("Page Table entry isn't present and attempted to free");
                let virt_addr = phys_to_virt(addr);
                let table = &mut *(virt_addr.into_ptr::<ArchPageTable>());
                table.free(level);
            }
            self.deallocate();
        }
    }

    /// deallocates a page table entry and invalidates it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    unsafe fn deallocate(&mut self) {
        if let Some(frame) = self.frame() {
            self.set_present(false);

            unsafe {
                frame_allocator::deallocate_frame(frame)
                    .expect("Failed to deallocate a page table entry")
            };
        }
    }

    #[inline(always)]
    /// Attempts to map this entry to a new page table or if it is already mapped returns the page table it was mapped to.
    ///
    /// Safety: should be inheirtly safe, however the pointer it returns could be invalid if the entry is bad, if the entry isn't present it will allocate new memory for it.
    fn map(&mut self, user_accessible: bool) -> Result<NonNull<ArchPageTable>, MapToError> {
        if let Some(addr) = self.addr() {
            let virt_addr = phys_to_virt(addr);
            let entry_ptr = virt_addr.into_ptr::<ArchPageTable>();
            // When you map a page that is user accessible all levels must be accessible.
            if user_accessible {
                self.set_user_accessible(true);
            }

            Ok(NonNull::new(entry_ptr).expect("PageTable entry mapped to null"))
        } else {
            let frame = frame_allocator::allocate_frame()?;

            let addr = frame.addr();
            let virt_addr = phys_to_virt(addr);
            let table_ptr = virt_addr.into_ptr::<ArchPageTable>();
            unsafe { (*table_ptr).zeroize() };

            self.set_addr(addr);
            self.set_present(true);
            self.set_writable(true);
            self.set_user_accessible(user_accessible);

            Ok(NonNull::new(table_ptr).expect("allocated PageTable entry is null"))
        }
    }

    /// if an entry is mapped returns the PageTable or the Frame(as a PageTable) it is mapped to
    fn mapped_to(&self) -> Option<NonNull<ArchPageTable>> {
        if let Some(addr) = self.addr() {
            let virt_addr = phys_to_virt(addr);
            let entry_ptr = virt_addr.into_ptr::<ArchPageTable>();

            return Some(NonNull::new(entry_ptr).expect("PageTable entry is mapped to null"));
        }

        None
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct ArchPageTable {
    entries: [Entry; ENTRY_COUNT],
}

impl PageTableOps for ArchPageTable {
    unsafe fn sync_higher_half(&mut self) {
        self.copy_higher_half();
    }

    unsafe fn zeroize(&mut self) {
        self.zeroize();
    }

    fn get_frame_of(&self, page: Page) -> Option<Frame> {
        self.get_frame(page)
    }

    unsafe fn deallocate(&mut self) {
        unsafe { self.free(4) };
    }

    fn finish_ops(&mut self, _pages: IterPage) {
        todo!("TLB Shootdown")
    }

    fn map_range(
        &mut self,
        pages: IterPage,
        frames: FrameIter,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        for (page, frame) in pages.zip(frames) {
            unsafe { self.map_to(page, frame, flags)? };
        }

        Ok(())
    }

    unsafe fn unmap_range<F>(
        &mut self,
        pages: IterPage,
        mut with_each: F,
        lazy: bool,
    ) -> Result<(), MapToError>
    where
        F: FnMut(Page, Frame),
    {
        for page in pages {
            let frame = unsafe { self.unmap(page) };
            match frame {
                Ok(frame) => with_each(page, frame),
                Err(MapToError::NotMapped) if lazy => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    unsafe fn set_flags_range(
        &mut self,
        pages: IterPage,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        for page in pages {
            match self
                .get_entry(page)
                .map(|mut e| unsafe { e.as_mut() })
                .filter(|e| e.present())
            {
                Some(entry) => {
                    let mut empty_entry = Entry::new_from_flags(flags, true);
                    empty_entry.set_pagenum(entry.pagenum());
                    *entry = empty_entry;
                }
                None if flags.contains(PageEntryFlags::_IS_LAZY) => {}
                None => return Err(MapToError::NotMapped),
            }
        }
        Ok(())
    }
}

impl ArchPageTable {
    fn zeroize(&mut self) {
        self.entries.fill(const { unsafe { core::mem::zeroed() } });
    }

    /// copies the higher half entries of the current pml4 to this page table
    fn copy_higher_half(&mut self) {
        unsafe {
            self.entries[HIGHER_HALF_ENTRY..ENTRY_COUNT].clone_from_slice(
                &current_kernel().as_ref().entries[HIGHER_HALF_ENTRY..ENTRY_COUNT],
            )
        }
    }
    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    unsafe fn free(&mut self, level: u8) {
        unsafe {
            // if the table is the pml4 we need not to free the higher half
            // because it is shared with other tables
            let last_entry = if level >= 4 {
                HIGHER_HALF_ENTRY
            } else {
                ENTRY_COUNT
            };

            for entry in &mut self.entries[0..last_entry] {
                if entry.0 != 0 {
                    entry.free(level - 1);
                }
            }
        }
    }

    #[inline]
    /// Maps a virtual `Page` to physical `Frame`.
    unsafe fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) = translate(page.addr());

        let empty_entry = Entry::new_from_flags(flags, true);
        let user_accessible = empty_entry.user_accessible();

        unsafe {
            let mut level_3_table = self[level_4_index].map(user_accessible)?;
            let mut level_2_table = level_3_table.as_mut()[level_3_index].map(user_accessible)?;
            let mut level_1_table = level_2_table.as_mut()[level_2_index].map(user_accessible)?;

            let entry = &mut level_1_table.as_mut()[level_1_index];
            if entry.addr().is_some() {
                return Err(MapToError::AlreadyMapped);
            }

            *entry = empty_entry.with_addr(frame.addr());
        }
        Ok(())
    }

    #[inline]
    /// Gets the frame page points to.
    fn get_frame(&self, page: Page) -> Option<Frame> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) = translate(page.addr());
        unsafe {
            let level_3_table = self[level_4_index].mapped_to()?;
            let level_2_table = level_3_table.as_ref()[level_3_index].mapped_to()?;
            let level_1_table = level_2_table.as_ref()[level_2_index].mapped_to()?;

            let entry = &level_1_table.as_ref()[level_1_index];
            entry.frame()
        }
    }

    #[inline]
    /// Gets a pointer to the entry a given `page` is mapped to.
    pub fn get_entry(&self, page: Page) -> Option<NonNull<Entry>> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) = translate(page.addr());
        unsafe {
            let level_3_table = self[level_4_index].mapped_to()?;
            let level_2_table = level_3_table.as_ref()[level_3_index].mapped_to()?;
            let mut level_1_table = level_2_table.as_ref()[level_2_index].mapped_to()?;

            Some(NonNull::from_mut(
                &mut level_1_table.as_mut()[level_1_index],
            ))
        }
    }

    #[inline]
    /// Unmaps a page without flushing the cache or freeing the frame.
    unsafe fn unmap(&mut self, page: Page) -> Result<Frame, MapToError> {
        let entry = self.get_entry(page);
        if let Some(mut entry) = entry {
            let entry = unsafe { entry.as_mut() };
            let frame = entry.frame().ok_or(MapToError::NotMapped)?;
            entry.zeroize();
            Ok(frame)
        } else {
            Err(MapToError::NotMapped)
        }
    }
}

impl Index<usize> for ArchPageTable {
    type Output = Entry;
    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

impl IndexMut<usize> for ArchPageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.entries[index]
    }
}

/// Returns the current pml4 from cr3
pub fn current_kernel() -> NonNull<ArchPageTable> {
    let phys_addr: usize;
    unsafe {
        asm!("mov {}, cr3", out(reg) phys_addr, options(nostack, nomem, preserves_flags));
    }

    let phys_addr = PhysAddr::from(phys_addr);
    let ptr = phys_to_virt(phys_addr).into_ptr::<ArchPageTable>();
    unsafe { NonNull::new_unchecked(ptr) }
}

/// returns the current pml4 from cr3
/// equalivent to [`current_kernel`] in x86_64
pub fn current_user() -> NonNull<ArchPageTable> {
    current_kernel()
}

/// Sets the current higher half Page Table address to `addr`
pub unsafe fn set_current_kernel(addr: PhysAddr) {
    unsafe {
        asm!("mov cr3, {}", in(reg) addr.raw(), options(nostack));
    }
}
