use bitflags::bitflags;

use crate::{
    arch::aarch64::registers::SYS_MAIR,
    limine::HHDM,
    memory::{
        frame_allocator::{self, Frame, FramePtr},
        paging::{EntryFlags, MapToError, Page},
    },
    PhysAddr, VirtAddr,
};
use core::{
    arch::asm,
    ops::{Index, IndexMut},
};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct ArchEntryFlags: u64 {
        const PRESENT = 1 << 0;
        const TABLE_DESC = 1 << 1;
        const ACCESS_FLAG = 1 << 10;

        const MAIR1 = 1 << 2;
        const MAIR2 = 1 << 3;
        const MAIR3 = 1 << 4;

        const NON_SECURE = 1 << 5;
        const AP_LOWER = 1 << 6;
        const AP_HIGHER = 1 << 7;
        const NO_EXEC_PRIV = 1 << 53;
        const NO_EXEC_UNPRIV = 1 << 54;
    }
}

impl From<EntryFlags> for ArchEntryFlags {
    fn from(value: EntryFlags) -> Self {
        let mut flags: ArchEntryFlags =
            // MAIR index 0 for now
            ArchEntryFlags::PRESENT | ArchEntryFlags::TABLE_DESC | ArchEntryFlags::ACCESS_FLAG;
        if !value.contains(EntryFlags::WRITE) {
            // read-only flag
            flags |= ArchEntryFlags::AP_HIGHER;
        }

        if value.contains(EntryFlags::USER_ACCESSIBLE) {
            flags |= ArchEntryFlags::AP_LOWER;
        }
        flags
    }
}

#[inline(always)]
const fn l0_index(addr: VirtAddr) -> usize {
    (addr >> 39) & 0x1FF
}

#[inline(always)]
const fn l1_index(addr: VirtAddr) -> usize {
    (addr >> 30) & 0x1FF
}
#[inline(always)]
const fn l2_index(addr: VirtAddr) -> usize {
    (addr >> 21) & 0x1FF
}
#[inline(always)]
const fn l3_index(addr: VirtAddr) -> usize {
    (addr >> 12) & 0x1FF
}

/// translates a
fn translate(addr: VirtAddr) -> (bool, usize, usize, usize, usize) {
    let is_higher_half = (addr >> 63) & 1 == 1;
    (
        is_higher_half,
        l0_index(addr),
        l1_index(addr),
        l2_index(addr),
        l3_index(addr),
    )
}

#[derive(Clone, Debug)]
/// A page table's entry
pub struct Entry(u64);

impl Entry {
    fn flags(&self) -> ArchEntryFlags {
        ArchEntryFlags::from_bits_retain(self.0)
    }

    fn frame(&self) -> Option<Frame> {
        let flags = self.flags();
        if flags.contains(ArchEntryFlags::PRESENT)
            || flags.contains(ArchEntryFlags::TABLE_DESC)
            || flags.contains(ArchEntryFlags::ACCESS_FLAG)
        {
            return Some(Frame::containing_address(
                // TODO: simplify this
                // 47 bits set after the first 12 bits
                (self.0 & (((1 << 47) - 1) << 12)) as usize,
            ));
        }
        None
    }

    const fn new(flags: ArchEntryFlags, addr: PhysAddr) -> Self {
        Self(addr as u64 | flags.bits())
    }

    const fn set(&mut self, flags: ArchEntryFlags, addr: PhysAddr) {
        *self = Self::new(flags, addr)
    }

    #[inline(always)]
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    fn map(&mut self) -> Result<&'static mut PageTable, MapToError> {
        let flags = ArchEntryFlags::TABLE_DESC | ArchEntryFlags::PRESENT;
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
}

#[derive(Debug, Clone)]
#[repr(C)]
#[repr(align(0x1000))]
pub struct PageTable([Entry; 512]);

impl Index<usize> for PageTable {
    type Output = Entry;
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for PageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

/// Returns the current higher half root table
pub unsafe fn current_higher_root_table() -> FramePtr<PageTable> {
    let ttbr1_el1: PhysAddr;
    unsafe {
        asm!("mrs {}, ttbr1_el1", out(reg) ttbr1_el1);
        let frame = Frame::containing_address(ttbr1_el1);
        frame.into_ptr()
    }
}

/// Returns the current lower half root table
pub unsafe fn current_lower_root_table() -> FramePtr<PageTable> {
    let ttbr0_el1: PhysAddr;
    unsafe {
        asm!("mrs {}, ttbr0_el1", out(reg) ttbr0_el1);
        let frame = Frame::containing_address(ttbr0_el1);
        frame.into_ptr()
    }
}

/// sets the current higher half Page Table to `page_table`
pub unsafe fn set_current_higher_page_table(page_table: FramePtr<PageTable>) {
    let ttbr1_el1: PhysAddr = page_table.phys_addr();
    unsafe {
        asm!("
        msr ttbr1_el1, {}
        tlbi VMALLE1
        dsb ISH
        isb", in(reg) ttbr1_el1);
        let mair = SYS_MAIR;
        mair.sync();
    }
}

// TODO: maybe use traits here
impl PageTable {
    pub fn zeroize(&mut self) {
        *self = unsafe { core::mem::zeroed() };
    }

    /// copies the higher half entries of the current pml4 to this page table
    pub fn copy_higher_half(&mut self) {
        // not needed in aarch64 because the higher half lives in another register anyways
    }

    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    pub unsafe fn free(&mut self, level: u8) {
        for entry in &mut self.0 {
            if entry.flags().contains(ArchEntryFlags::PRESENT) {
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
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.start_address);
        let flags: ArchEntryFlags = flags.into();
        let l1 = self[l0_index].map()?;
        let l2 = l1[l1_index].map()?;
        let l3 = l2[l2_index].map()?;
        let entry = &mut l3[l3_index];

        // TODO: stress test this
        debug_assert!(
                    entry.frame().is_none(),
                    "entry {:?} already has a frame {:?}, but we're trying to map it to {:?} with page {:#x}",
                    entry,
                    entry.frame(),
                    frame,
                    page.start_address
                );

        entry.set(flags, frame.start_address());
        Ok(())
    }

    /// gets the frame page points to
    pub fn get_frame(&self, page: Page) -> Option<Frame> {
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.start_address);
        let l1 = self[l0_index].mapped_to()?;
        let l2 = l1[l1_index].mapped_to()?;
        let l3 = l2[l2_index].mapped_to()?;
        let entry = &l3[l3_index];

        entry.frame()
    }

    /// get a mutable reference to the entry for a given page
    fn get_entry(&self, page: Page) -> Option<&mut Entry> {
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.start_address);
        let l1 = self[l0_index].mapped_to()?;
        let l2 = l1[l1_index].mapped_to()?;
        let l3 = l2[l2_index].mapped_to()?;

        Some(&mut l3[l3_index])
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

/// Maps architecture specific devices such as the UART serial in aarch64
pub unsafe fn map_devices(table: &mut PageTable) -> Result<(), MapToError> {
    let flags = EntryFlags::WRITE;
    table.map_to(
        Page::containing_address(*HHDM | *super::cpu::PL011BASE),
        Frame::containing_address(*super::cpu::PL011BASE),
        flags,
    )?;
    Ok(())
}
