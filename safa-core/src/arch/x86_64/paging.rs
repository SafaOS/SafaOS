use bitflags::bitflags;
use core::cell::SyncUnsafeCell;
use core::fmt::Debug;
use core::ops::IndexMut;
use core::{arch::asm, ops::Index};

use crate::VirtAddr;
use crate::arch::x86_64::interrupts::apic;
use crate::arch::x86_64::tlb;
use crate::memory::paging::{Page, PageEntryFlags, PageTableOps};
use crate::memory::vmm::{VMMAllocError, VirtualMemoryManager};
use crate::{
    PhysAddr,
    memory::{
        frame_allocator::{self, Frame, FramePtr},
        paging::MapToError,
    },
};

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
    let addr = addr.into_raw();
    (
        p1_index(addr),
        p2_index(addr),
        p3_index(addr),
        p4_index(addr),
    )
}

#[derive(Clone)]
/// A page table's entry
pub struct Entry(usize);
impl Debug for Entry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Entry")
            .field(&format_args!("{:#x}", self.0))
            .field(&self.flags())
            .finish()
    }
}

impl Entry {
    /// Returns the physical address associated with this entry, even if it is not mapped.
    fn phys_addr(&self) -> PhysAddr {
        PhysAddr::from(self.0 & 0x000F_FFFF_FFFF_F000)
    }

    #[inline(always)]
    fn zeroize(&mut self) {
        self.0 = 0;
    }

    /// Returns the frame associated with this entry, if it is mapped.
    fn frame(&self) -> Option<Frame> {
        if self.flags().contains(ArchEntryFlags::PRESENT) {
            return Some(Frame::containing_address(self.phys_addr()));
        }
        None
    }

    fn flags(&self) -> ArchEntryFlags {
        ArchEntryFlags::from_bits_truncate(self.0 as u64)
    }

    const fn new(flags: ArchEntryFlags, addr: PhysAddr) -> Self {
        Self(addr.into_raw() | flags.bits() as usize)
    }

    #[inline(always)]
    fn set(&mut self, flags: ArchEntryFlags, addr: PhysAddr) {
        *self = Self::new(flags, addr)
    }

    /// deallocates an entry depending on it's level if it is 1 it should just deallocate the frame
    /// otherwise treat the frame as a page table and deallocate it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    unsafe fn free(&mut self, level: u8) {
        unsafe {
            let frame = self.frame().unwrap();

            if level != 0 {
                let table = &mut *(frame.virt_addr().into_ptr::<ArchPageTable>());
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
            frame_allocator::deallocate_frame(frame);
            self.set(ArchEntryFlags::empty(), PhysAddr::null());
        }
    }

    #[inline(always)]
    /// changes the entry flags to `flags`
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    fn map(&mut self) -> Result<&'static mut ArchPageTable, MapToError> {
        let flags =
            ArchEntryFlags::PRESENT | ArchEntryFlags::WRITABLE | ArchEntryFlags::USER_ACCESSIBLE;

        if let Some(frame) = self.frame() {
            let addr = frame.start_address();

            self.set(flags, addr);
            let virt_addr = frame.virt_addr();
            let entry_ptr = virt_addr.into_ptr::<ArchPageTable>();

            Ok(unsafe { &mut *(entry_ptr) })
        } else {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

            let addr = frame.start_address();
            self.set(flags, addr);

            let virt_addr = frame.virt_addr();
            let table_ptr = virt_addr.into_ptr::<ArchPageTable>();

            Ok(unsafe {
                (*table_ptr).zeroize();
                &mut *(table_ptr)
            })
        }
    }

    /// if an entry is mapped returns the PageTable or the Frame(as a PageTable) it is mapped to
    fn mapped_to(&self) -> Option<&'static mut ArchPageTable> {
        if let Some(frame) = self.frame() {
            let virt_addr = frame.virt_addr();
            let entry_ptr = virt_addr.into_ptr::<ArchPageTable>();

            return Some(unsafe { &mut *entry_ptr });
        }

        None
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct ArchEntryFlags: u64 {
        const PRESENT         = 1;
        const WRITABLE        = 1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const PWT             = 1 << 3;
        const PCD             = 1 << 4;
        const ACCESSED        = 1 << 5;
        const DIRTY           = 1 << 6;
        const HUGE_PAGE       = 1 << 7;
        const PAT             = 1 << 7;
        const GLOBAL          = 1 << 8;
        const PAT_OTHER       = 1 << 12;
        const NO_EXECUTE      = 1 << 63;
    }
}

impl ArchEntryFlags {
    pub const fn from_flags_outer_levels(value: PageEntryFlags) -> Self {
        let mut this = ArchEntryFlags::PRESENT;
        if value.contains(PageEntryFlags::WRITE) {
            this = this.union(ArchEntryFlags::WRITABLE);
        }

        if value.contains(PageEntryFlags::DEVICE_UNCACHEABLE) {
            this = this.union(ArchEntryFlags::PCD);
        }

        if value.contains(PageEntryFlags::USER_ACCESSIBLE) {
            this = this.union(ArchEntryFlags::USER_ACCESSIBLE);
        }

        if value.contains(PageEntryFlags::DISABLE_EXEC) {
            this = this.union(ArchEntryFlags::NO_EXECUTE);
        }

        this
    }
}

impl From<PageEntryFlags> for ArchEntryFlags {
    fn from(value: PageEntryFlags) -> Self {
        let mut this = Self::from_flags_outer_levels(value);

        if value.contains(PageEntryFlags::FRAMEBUFFER_CACHED) {
            this |= ArchEntryFlags::PAT | ArchEntryFlags::PWT;
        }
        this
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

    fn finish_ops(&mut self, pages: crate::memory::paging::IterPage) {
        unsafe { tlb::flush_cache_range(self, pages.current_addr(), pages.end_addr()) };
    }

    fn map_range(
        &mut self,
        pages: crate::memory::paging::IterPage,
        frames: frame_allocator::FrameIter,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        for (page, frame) in pages.zip(frames) {
            unsafe { self.map_to(page, frame, flags)? };
        }

        Ok(())
    }

    unsafe fn unmap_range<F>(
        &mut self,
        pages: crate::memory::paging::IterPage,
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
        pages: crate::memory::paging::IterPage,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        for page in pages {
            match self.get_entry(page).and_then(|e| Some((e.frame()?, e))) {
                Some((f, entry)) => {
                    entry.set(
                        flags.difference(PageEntryFlags::IS_LAZY).into(),
                        f.phys_addr(),
                    );
                }
                None if flags.contains(PageEntryFlags::IS_LAZY) => {}
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
                &current_higher_root_table().entries[HIGHER_HALF_ENTRY..ENTRY_COUNT],
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

        let final_flags: ArchEntryFlags = flags.into();

        let level_3_table = self[level_4_index].map()?;

        let level_2_table = level_3_table[level_3_index].map()?;

        let level_1_table = level_2_table[level_2_index].map()?;

        let entry = &mut level_1_table[level_1_index];
        if entry.frame().is_some() {
            return Err(MapToError::AlreadyMapped);
        }

        *entry = Entry::new(final_flags, frame.start_address());
        Ok(())
    }

    #[inline]
    /// Gets the frame page points to.
    fn get_frame(&self, page: Page) -> Option<Frame> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) = translate(page.addr());
        let level_3_table = self[level_4_index].mapped_to()?;
        let level_2_table = level_3_table[level_3_index].mapped_to()?;
        let level_1_table = level_2_table[level_2_index].mapped_to()?;

        let entry = &level_1_table[level_1_index];

        entry.frame()
    }

    #[inline]
    /// get a mutable reference to the entry for a given page
    pub fn get_entry(&self, page: Page) -> Option<&mut Entry> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) = translate(page.addr());
        let level_3_table = self[level_4_index].mapped_to()?;
        let level_2_table = level_3_table[level_3_index].mapped_to()?;
        let level_1_table = level_2_table[level_2_index].mapped_to()?;

        Some(&mut level_1_table[level_1_index])
    }

    #[inline]
    /// Unmaps a page without flushing the cache or freeing the frame.
    unsafe fn unmap(&mut self, page: Page) -> Result<Frame, MapToError> {
        let entry = self.get_entry(page);
        if let Some(entry) = entry {
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

/// returns the current pml4 from cr3
pub unsafe fn current_higher_root_table() -> FramePtr<ArchPageTable> {
    let phys_addr: usize;
    unsafe {
        asm!("mov {}, cr3", out(reg) phys_addr);
    }

    let phys_addr = PhysAddr::from(phys_addr);
    let frame = Frame::containing_address(phys_addr);
    let ptr = unsafe { frame.into_ptr() };
    ptr
}

/// returns the current pml4 from cr3
/// equalivent to [`current_higher_root_table`] in x86_64
pub unsafe fn current_lower_root_table() -> FramePtr<ArchPageTable> {
    unsafe { current_higher_root_table() }
}

pub static CURRENT_RING0_PAGE_TABLE: SyncUnsafeCell<PhysAddr> =
    SyncUnsafeCell::new(PhysAddr::null());

pub(super) unsafe fn set_current_page_table_phys(phys_addr: PhysAddr) {
    unsafe {
        asm!("mov cr3, rax", in("rax") phys_addr.into_raw());
    }
}
/// sets the current higher half Page Table to `page_table`
pub unsafe fn set_current_higher_page_table(page_table: FramePtr<ArchPageTable>) {
    let phys_addr = page_table.phys_addr();
    unsafe {
        set_current_page_table_phys(phys_addr);
        *CURRENT_RING0_PAGE_TABLE.get() = phys_addr;
    }
}

/// Maps architecture specific devices such as the UART serial in aarch64
pub unsafe fn map_devices(vmm: &mut VirtualMemoryManager) -> Result<(), VMMAllocError> {
    use crate::memory::HHDM;

    apic::map_apic(vmm)?;
    // a hack to handle sharing the higher half in x86_64
    let (hhdm_start, end) = (HHDM, HHDM + 0x400000000000);

    let (_, _, _, heap_p4_index) = translate(hhdm_start);
    let (_, _, _, heap_end_p4_index) = translate(end);

    let table = unsafe { vmm.table_mut() };
    for entry in &mut table.inner_mut().entries[heap_p4_index..heap_end_p4_index] {
        entry.map()?;
    }

    crate::serial!("Mapped entries from {heap_p4_index} to {heap_end_p4_index}...\n");
    Ok(())
}
