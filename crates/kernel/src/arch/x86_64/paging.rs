use bitflags::bitflags;
use core::fmt::Debug;
use core::ops::IndexMut;
use core::{arch::asm, ops::Index};

use crate::limine::HHDM;
use crate::memory::paging::{EntryFlags, Page};
use crate::VirtAddr;
use crate::{
    memory::{
        frame_allocator::{self, Frame, FramePtr},
        paging::{MapToError, PAGE_SIZE},
    },
    PhysAddr,
};

const ENTRY_COUNT: usize = 512;
const HIGHER_HALF_ENTRY: usize = 256;

const fn p4_index(addr: VirtAddr) -> usize {
    (addr >> 39) & 0x1FF
}
const fn p3_index(addr: VirtAddr) -> usize {
    (addr >> 30) & 0x1FF
}
const fn p2_index(addr: VirtAddr) -> usize {
    (addr >> 21) & 0x1FF
}
const fn p1_index(addr: VirtAddr) -> usize {
    (addr >> 12) & 0x1FF
}

fn translate(addr: VirtAddr) -> (usize, usize, usize, usize) {
    (
        p1_index(addr),
        p2_index(addr),
        p3_index(addr),
        p4_index(addr),
    )
}

#[derive(Clone)]
/// A page table's entry
pub struct Entry(PhysAddr);
impl Debug for Entry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Entry")
            .field(&format_args!("{:#x}", self.0))
            .field(&self.flags())
            .finish()
    }
}

impl Entry {
    fn frame(&self) -> Option<Frame> {
        if self.flags().contains(ArchEntryFlags::PRESENT) {
            // FIXME: real hardware problem here
            // TODO: figure out more info about the max physical address width
            return Some(Frame::containing_address(self.0 & 0x000F_FFFF_FFFF_F000));
        }
        None
    }

    fn flags(&self) -> ArchEntryFlags {
        ArchEntryFlags::from_bits_truncate(self.0 as u64)
    }

    const fn new(flags: ArchEntryFlags, addr: PhysAddr) -> Self {
        Self(addr | flags.bits() as usize)
    }

    const fn set(&mut self, flags: ArchEntryFlags, addr: PhysAddr) {
        *self = Self::new(flags, addr)
    }

    /// deallocates an entry depending on it's level if it is 1 it should just deallocate the frame
    /// otherwise treat the frame as a page table and deallocate it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    unsafe fn free(&mut self, level: u8) {
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
    unsafe fn deallocate(&mut self) {
        if let Some(frame) = self.frame() {
            frame_allocator::deallocate_frame(frame);
            self.set(ArchEntryFlags::empty(), 0);
        }
    }

    #[inline(always)]
    /// changes the entry flags to `flags`
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    fn map(&mut self, flags: ArchEntryFlags) -> Result<&'static mut PageTable, MapToError> {
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
    fn mapped_to(&self) -> Option<&'static mut PageTable> {
        if let Some(frame) = self.frame() {
            let virt_addr = frame.virt_addr();
            let entry_ptr = virt_addr as *mut PageTable;

            return Some(unsafe { &mut *entry_ptr });
        }

        None
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct ArchEntryFlags: u64 {
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

impl From<EntryFlags> for ArchEntryFlags {
    fn from(value: EntryFlags) -> Self {
        let mut this = ArchEntryFlags::PRESENT;
        if value.contains(EntryFlags::WRITE) {
            this |= ArchEntryFlags::WRITABLE;
        }

        if value.contains(EntryFlags::USER_ACCESSIBLE) {
            this |= ArchEntryFlags::USER_ACCESSIBLE;
        }

        if value.contains(EntryFlags::DISABLE_EXEC) {
            this |= ArchEntryFlags::NO_EXECUTE;
        }

        this
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct PageTable {
    entries: [Entry; ENTRY_COUNT],
}

impl PageTable {
    pub fn zeroize(&mut self) {
        self.entries.fill(const { unsafe { core::mem::zeroed() } });
    }

    /// copies the higher half entries of the current pml4 to this page table
    pub fn copy_higher_half(&mut self) {
        unsafe {
            self.entries[HIGHER_HALF_ENTRY..ENTRY_COUNT].clone_from_slice(
                &current_higher_root_table().entries[HIGHER_HALF_ENTRY..ENTRY_COUNT],
            )
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

    /// maps a virtual `Page` to physical `Frame`
    pub unsafe fn map_to(
        &mut self,
        page: Page,
        frame: Frame,
        flags: EntryFlags,
    ) -> Result<(), MapToError> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) =
            translate(page.virt_addr());
        let flags: ArchEntryFlags = flags.into();
        let level_3_table = self[level_4_index].map(flags)?;

        let level_2_table = level_3_table[level_3_index].map(flags)?;

        let level_1_table = level_2_table[level_2_index].map(flags)?;

        let entry = &mut level_1_table[level_1_index];
        // TODO: stress test this
        debug_assert!(
                entry.frame().is_none(),
                "entry {:?} already has a frame {:?}, but we're trying to map it to {:?} with page {page:?}",
                entry,
                entry.frame(),
                frame,
            );

        *entry = Entry::new(flags, frame.start_address());
        Ok(())
    }

    /// gets the frame page points to
    pub fn get_frame(&self, page: Page) -> Option<Frame> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) =
            translate(page.virt_addr());
        let level_3_table = self[level_4_index].mapped_to()?;
        let level_2_table = level_3_table[level_3_index].mapped_to()?;
        let level_1_table = level_2_table[level_2_index].mapped_to()?;

        let entry = &level_1_table[level_1_index];

        entry.frame()
    }

    /// get a mutable reference to the entry for a given page
    fn get_entry(&self, page: Page) -> Option<&mut Entry> {
        let (level_1_index, level_2_index, level_3_index, level_4_index) =
            translate(page.virt_addr());
        let level_3_table = self[level_4_index].mapped_to()?;
        let level_2_table = level_3_table[level_3_index].mapped_to()?;
        let level_1_table = level_2_table[level_2_index].mapped_to()?;

        Some(&mut level_1_table[level_1_index])
    }

    /// unmaps a page
    pub unsafe fn unmap(&mut self, page: Page) {
        let entry = self.get_entry(page);
        debug_assert!(entry.is_some());
        if let Some(entry) = entry {
            unsafe { entry.deallocate() };
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
pub unsafe fn current_higher_root_table() -> FramePtr<PageTable> {
    let phys_addr: PhysAddr;
    unsafe {
        asm!("mov {}, cr3", out(reg) phys_addr);
    }

    let frame = Frame::containing_address(phys_addr);
    let ptr = frame.into_ptr();
    ptr
}

/// returns the current pml4 from cr3
/// equalivent to [`current_higher_root_table`] in x86_64
pub unsafe fn current_lower_root_table() -> FramePtr<PageTable> {
    current_higher_root_table()
}

/// sets the current higher half Page Table to `page_table`
pub unsafe fn set_current_higher_page_table(page_table: FramePtr<PageTable>) {
    let phys_addr = page_table.phys_addr();
    unsafe {
        asm!("mov cr3, rax", in("rax") phys_addr);
    }
}

pub(super) const DEVICE_MAPPING_START: PhysAddr = 0xC000_0000;
pub(super) const DEVICE_MAPPING_END: PhysAddr = 0xFFFF_FFFF;
pub(super) const DEVICE_MAPPING_SIZE: usize = DEVICE_MAPPING_END - DEVICE_MAPPING_START;

/// Maps architecture specific devices such as the UART serial in aarch64
/// Maps from 0xC0000000 to 0xFFFFFFFF in x86_64
pub unsafe fn map_devices(table: &mut PageTable) -> Result<(), MapToError> {
    let start_virt_addr = DEVICE_MAPPING_START | *HHDM;
    table.map_contiguous_pages(
        start_virt_addr,
        DEVICE_MAPPING_START,
        DEVICE_MAPPING_SIZE / PAGE_SIZE,
        EntryFlags::WRITE,
    )?;
    Ok(())
}
