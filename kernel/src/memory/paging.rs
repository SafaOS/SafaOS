const ENTRY_COUNT: usize = 512;
const HIGHER_HALF_ENTRY: usize = 256;

pub const PAGE_SIZE: usize = 4096;
use crate::{
    hddm,
    memory::{translate, PhysAddr},
};
use bitflags::bitflags;
use core::{
    arch::asm,
    ops::{Deref, DerefMut, Index, IndexMut},
};

use crate::memory::frame_allocator::Frame;

use super::{align_down, frame_allocator, VirtAddr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub start_address: VirtAddr,
}
#[derive(Debug, Clone)]
pub struct IterPage {
    pub start: Page,
    pub end: Page,
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
        assert!(start.start_address < end.start_address);
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

#[derive(Debug, Clone)]
pub struct Entry(PhysAddr);
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
    /// &mut self becomes invaild after use
    pub unsafe fn free(&mut self, level: u8) {
        let frame = self.frame().unwrap();

        if level == 0 {
            frame_allocator::deallocate_frame(frame);
            return;
        }
        let table = &mut *((frame.start_address | hddm()) as *mut PageTable);
        table.free(level)
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
        for entry in &mut self.entries {
            entry.0 = 0;
        }
    }

    /// copies the higher half entries of the current pml4 to this page table
    pub fn copy_higher_half(&mut self) {
        unsafe {
            self.entries[HIGHER_HALF_ENTRY..ENTRY_COUNT]
                .clone_from_slice(&current_root_table().entries[HIGHER_HALF_ENTRY..ENTRY_COUNT])
        }
    }
    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    /// unsafe because self becomes invaild after use
    pub unsafe fn free(&mut self, level: u8) {
        for entry in &mut self.entries[0..HIGHER_HALF_ENTRY] {
            if entry.0 != 0 {
                entry.free(level - 1);
            }
        }

        let table_addr = self as *mut PageTable as VirtAddr;

        let frame = Frame::containing_address(table_addr - hddm());
        frame_allocator::deallocate_frame(frame)
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
pub unsafe fn current_root_table() -> &'static mut PageTable {
    let phys_addr: PhysAddr;
    unsafe {
        asm!("mov {}, cr3", out(reg) phys_addr);
    }
    let frame = Frame::containing_address(phys_addr);

    let virt_addr = frame.start_address | hddm();

    &mut *(virt_addr as *mut PageTable)
}

#[derive(Debug)]
pub enum MapToError {
    FrameAllocationFailed,
}

impl Entry {
    #[inline(always)]
    /// changes the entry flags to `flags`
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    #[cfg(target_arch = "x86_64")]
    fn map(&mut self, flags: EntryFlags) -> Result<&'static mut PageTable, MapToError> {
        use crate::hddm;

        if self.is_mapped() {
            let addr = self.frame().unwrap().start_address;

            self.set(flags, addr);
            let virt_addr = addr | hddm();
            let entry_ptr = virt_addr as *mut PageTable;

            Ok(unsafe { &mut *(entry_ptr) })
        } else {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

            let addr = frame.start_address;
            self.set(flags, addr);

            let virt_addr = addr | hddm();
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
        if self.is_mapped() {
            let addr = self.frame().unwrap().start_address;
            let virt_addr = addr | hddm();
            let entry_ptr = virt_addr as *mut PageTable;

            return Some(unsafe { &mut *entry_ptr });
        }

        None
    }

    #[inline]
    pub fn is_mapped(&self) -> bool {
        self.flags().contains(EntryFlags::PRESENT)
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
        if let Some(frame) = entry.frame().filter(|x| x != &frame) {
            frame_allocator::deallocate_frame(frame);
        }

        *entry = Entry::new(flags, frame.start_address);
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

    /// unmap page and all of it's entries
    pub fn unmap(&mut self, page: Page) {
        self.get_frame(page)
            .inspect(|x| frame_allocator::deallocate_frame(*x));
    }
}

/// allocates a pml4 and returns its physical address
fn allocate_pml4<'a>() -> Result<&'a mut PageTable, MapToError> {
    let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
    let virt_start_addr = frame.start_address | hddm();
    let table = unsafe { &mut *(virt_start_addr as *mut PageTable) };

    table.zeroize();
    table.copy_higher_half();

    Ok(table)
}

#[repr(C)]
/// A wrapper around a Physically allocated page table
/// when the PhysPageTable is dropped it will free the whole page table so be careful with it
#[derive(Debug)]
pub struct PhysPageTable {
    inner: *mut PageTable,
}

impl Deref for PhysPageTable {
    type Target = PageTable;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner }
    }
}

impl DerefMut for PhysPageTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner }
    }
}

impl PhysPageTable {
    pub fn create() -> Result<Self, MapToError> {
        let page_table = allocate_pml4()?;
        Ok(Self {
            inner: page_table as *mut PageTable,
        })
    }

    /// creates a new PhysPageTable from the current pml4 table
    /// takes ownership of the current pml4 table meaning it will free it when the PhysPageTable is dropped
    pub unsafe fn from_current() -> Self {
        let page_table = current_root_table() as *mut PageTable;
        Self { inner: page_table }
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
            let virt_addr = frame.start_address | hddm();
            self.map_to(page, frame, flags)?;

            unsafe {
                core::ptr::write_bytes(virt_addr as *mut u8, 0, PAGE_SIZE);
            }
        }

        Ok(())
    }

    /// Returns the root pml4 physical address
    /// # Safety
    /// The caller must ensure that the root pml4 is not freed, once [`self`] is dropped otherwise it will cause UB
    pub unsafe fn root_pml4(&self) -> PhysAddr {
        (self.inner as VirtAddr - hddm()) as PhysAddr
    }
}

impl Drop for PhysPageTable {
    fn drop(&mut self) {
        unsafe {
            self.free(4);
        }
    }
}
