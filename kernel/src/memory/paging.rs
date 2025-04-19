const ENTRY_COUNT: usize = 512;
const HIGHER_HALF_ENTRY: usize = 256;

pub const PAGE_SIZE: usize = 4096;
use crate::memory::{translate, PhysAddr};
use bitflags::bitflags;
use core::{
    arch::asm,
    fmt::{Debug, LowerHex},
    ops::{Deref, DerefMut, Index, IndexMut},
};
use thiserror::Error;

use crate::memory::frame_allocator::Frame;

use super::{
    align_down,
    frame_allocator::{self, FramePtr},
    VirtAddr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub start_address: VirtAddr,
}

impl LowerHex for Page {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Page({:#x})", self.start_address)
    }
}

#[derive(Debug, Clone)]
pub struct IterPage {
    start: Page,
    end: Page,
}

impl Page {
    pub const fn containing_address(address: VirtAddr) -> Self {
        Self {
            start_address: align_down(address, PAGE_SIZE),
        }
    }

    /// creates an iterator'able struct
    /// requires that start.start_address is smaller then end.start_address
    pub const fn iter_pages(start: Page, end: Page) -> IterPage {
        assert!(start.start_address <= end.start_address);
        IterPage { start, end }
    }
}

impl Iterator for IterPage {
    type Item = Page;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start.start_address < self.end.start_address {
            let page = self.start;

            self.start.start_address += PAGE_SIZE;
            Some(page)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct Entry(PhysAddr);
impl Debug for Entry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Entry")
            .field(&format_args!("{:#x}", self.0))
            .field(&self.flags())
            .finish()
    }
}
// address of the next table or physial frame in 0x000FFFFF_FFFFF000 (the fs is the address are the fs the rest are flags or reserved)

#[cfg(target_arch = "x86_64")]
impl Entry {
    pub fn frame(&self) -> Option<Frame> {
        if self.flags().contains(EntryFlags::PRESENT) {
            // FIXME: real hardware problem here
            // TODO: figure out more info about the max physical address width
            return Some(Frame::containing_address(self.0 & 0x000F_FFFF_FFFF_F000));
        }
        None
    }

    pub fn flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.0 as u64)
    }

    pub const fn new(flags: EntryFlags, addr: PhysAddr) -> Self {
        Self(addr | flags.bits() as usize)
    }

    pub const fn set(&mut self, flags: EntryFlags, addr: PhysAddr) {
        *self = Self::new(flags, addr)
    }

    /// deallocates an entry depending on it's level if it is 1 it should just deallocate the frame
    /// otherwise treat the frame as a page table and deallocate it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    pub unsafe fn free(&mut self, level: u8) {
        let frame = self.frame().unwrap();

        if level != 0 {
            let table = &mut *(frame.virt_addr() as *mut PageTable);
            table.free(level);
        }
        self.deallocate();
    }

    /// deallocates a page table entry and invalidates it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    pub unsafe fn deallocate(&mut self) {
        if let Some(frame) = self.frame() {
            frame_allocator::deallocate_frame(frame);
            self.set(EntryFlags::empty(), 0);
        }
    }
}

#[cfg(target_arch = "x86_64")]
bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct EntryFlags: u64 {
        const PRESENT =         1;
        const WRITABLE =        1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const WRITE_THROUGH =   1 << 3;
        const NO_CACHE =        1 << 4;
        const ACCESSED =        1 << 5;
        const DIRTY =           1 << 6;
        const HUGE_PAGE =       1 << 7;
        const GLOBAL =          1 << 8;
        const NO_EXECUTE =      1 << 63;
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct PageTable {
    pub entries: [Entry; ENTRY_COUNT],
}

impl PageTable {
    pub fn zeroize(&mut self) {
        self.entries.fill(const { unsafe { core::mem::zeroed() } });
    }

    /// copies the higher half entries of the current pml4 to this page table
    pub fn copy_higher_half(&mut self) {
        unsafe {
            self.entries[HIGHER_HALF_ENTRY..ENTRY_COUNT]
                .clone_from_slice(&current_root_table().entries[HIGHER_HALF_ENTRY..ENTRY_COUNT])
        }
    }
    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    pub unsafe fn free(&mut self, level: u8) {
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

impl Index<usize> for PageTable {
    type Output = Entry;
    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

impl IndexMut<usize> for PageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.entries[index]
    }
}

/// returns the current pml4 from cr3
#[cfg(target_arch = "x86_64")]
pub unsafe fn current_root_table() -> FramePtr<PageTable> {
    let phys_addr: PhysAddr;
    unsafe {
        asm!("mov {}, cr3", out(reg) phys_addr);
    }

    let frame = Frame::containing_address(phys_addr);
    let ptr = frame.into_ptr();
    ptr
}

#[derive(Debug, Clone, Copy, Error)]
pub enum MapToError {
    #[error("frame allocator: out of memory")]
    FrameAllocationFailed,
}

impl Entry {
    #[inline(always)]
    /// changes the entry flags to `flags`
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    #[cfg(target_arch = "x86_64")]
    fn map(&mut self, flags: EntryFlags) -> Result<&'static mut PageTable, MapToError> {
        if let Some(frame) = self.frame() {
            let addr = frame.start_address();

            self.set(flags, addr);
            let virt_addr = frame.virt_addr();
            let entry_ptr = virt_addr as *mut PageTable;

            Ok(unsafe { &mut *(entry_ptr) })
        } else {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

            let addr = frame.start_address();
            self.set(flags, addr);

            let virt_addr = frame.virt_addr();
            let table_ptr = virt_addr as *mut PageTable;

            Ok(unsafe {
                (*table_ptr).zeroize();
                &mut *(table_ptr)
            })
        }
    }

    /// if an entry is mapped returns the PageTable or the Frame(as a PageTable) it is mapped to
    #[inline]
    pub fn mapped_to(&self) -> Option<&'static mut PageTable> {
        if let Some(frame) = self.frame() {
            let virt_addr = frame.virt_addr();
            let entry_ptr = virt_addr as *mut PageTable;

            return Some(unsafe { &mut *entry_ptr });
        }

        None
    }
}

impl PageTable {
    /// maps a virtual `Page` to physical `Frame`
    pub fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) =
            translate(page.start_address);
        let level_3_table = self[level_4_index].map(flags)?;

        let level_2_table = level_3_table[level_3_index].map(flags)?;

        let level_1_table = level_2_table[level_2_index].map(flags)?;

        let entry = &mut level_1_table[level_1_index];
        // TODO: stress test this
        debug_assert!(
            entry.frame().is_none(),
            "entry {:?} already has a frame {:?}, but we're trying to map it to {:?} with page {:#x}",
            entry,
            entry.frame(),
            frame,
            page.start_address
        );

        *entry = Entry::new(flags, frame.start_address());
        Ok(())
    }

    /// gets the frame page points to
    pub fn get_frame(&mut self, page: Page) -> Option<Frame> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) =
            translate(page.start_address);
        let level_3_table = self[level_4_index].mapped_to()?;
        let level_2_table = level_3_table[level_3_index].mapped_to()?;
        let level_1_table = level_2_table[level_2_index].mapped_to()?;

        let entry = &level_1_table[level_1_index];

        entry.frame()
    }

    /// get a mutable reference to the entry for a given page
    pub fn get_entry(&self, page: Page) -> Option<&mut Entry> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) =
            translate(page.start_address);
        let level_3_table = self[level_4_index].mapped_to()?;
        let level_2_table = level_3_table[level_3_index].mapped_to()?;
        let level_1_table = level_2_table[level_2_index].mapped_to()?;

        Some(&mut level_1_table[level_1_index])
    }

    /// unmap page and all of it's entries
    pub fn unmap(&mut self, page: Page) {
        let entry = self.get_entry(page);
        debug_assert!(entry.is_some());
        if let Some(entry) = entry {
            unsafe { entry.deallocate() };
        }
    }
}

/// allocates a pml4 and returns its physical address
fn allocate_pml4<'a>() -> Result<FramePtr<PageTable>, MapToError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
    let mut table: FramePtr<PageTable> = unsafe { frame.into_ptr() };

    table.zeroize();
    table.copy_higher_half();

    Ok(table)
}

#[repr(C)]
/// A wrapper around a Physically allocated page table
/// when the PhysPageTable is dropped it will free the whole page table so be careful with it
#[derive(Debug)]
pub struct PhysPageTable {
    inner: FramePtr<PageTable>,
}

impl Deref for PhysPageTable {
    type Target = PageTable;
    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl DerefMut for PhysPageTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

impl PhysPageTable {
    pub fn create() -> Result<Self, MapToError> {
        let inner = allocate_pml4()?;
        Ok(Self { inner })
    }

    /// creates a new PhysPageTable from the current pml4 table
    /// takes ownership of the current pml4 table meaning it will free it when the PhysPageTable is dropped
    pub unsafe fn from_current() -> Self {
        let inner = current_root_table();
        Self { inner }
    }

    /// maps virtual pages from Page `from` to Page `to` with `flags` in `self`
    /// returns Err if any of the frames couldn't be allocated
    /// the mapped pages are zeroed
    pub fn alloc_map(
        &mut self,
        from: VirtAddr,
        to: VirtAddr,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        let from_page = Page::containing_address(from);
        let to_page = Page::containing_address(to);

        let iter = Page::iter_pages(from_page, to_page);

        for page in iter {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
            let virt_addr = frame.virt_addr();
            self.map_to(page, frame, flags)?;

            unsafe {
                core::ptr::write_bytes(virt_addr as *mut u8, 0, PAGE_SIZE);
            }
        }

        Ok(())
    }

    pub fn phys_addr(&self) -> PhysAddr {
        self.inner.phys_addr()
    }
}

impl Drop for PhysPageTable {
    fn drop(&mut self) {
        unsafe {
            self.free(4);
            // actually deallocating the page table
            let frame = self.inner.frame();
            frame_allocator::deallocate_frame(frame);
        }
    }
}

unsafe impl Send for PhysPageTable {}
