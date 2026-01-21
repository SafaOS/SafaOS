use bitflags::bitflags;

use crate::{
    PhysAddr, VirtAddr,
    arch::aarch64::{registers::SYS_MAIR, tlb},
    memory::{
        frame_allocator::{self, Frame, FramePtr},
        paging::{MapToError, Page, PageEntryFlags, PageTableOps},
        vmm::VirtualMemoryManager,
    },
};
use core::{
    arch::asm,
    cell::SyncUnsafeCell,
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

impl From<PageEntryFlags> for ArchEntryFlags {
    fn from(value: PageEntryFlags) -> Self {
        let mut flags: ArchEntryFlags =
            // MAIR index 0 for now
            ArchEntryFlags::PRESENT | ArchEntryFlags::TABLE_DESC | ArchEntryFlags::ACCESS_FLAG;

        if value.contains(PageEntryFlags::DEVICE_UNCACHEABLE) {
            flags |= ArchEntryFlags::MAIR1;
        } else if value.contains(PageEntryFlags::FRAMEBUFFER_CACHED) {
            flags |= ArchEntryFlags::MAIR2;
        }

        if !value.contains(PageEntryFlags::WRITE) {
            // read-only flag
            flags |= ArchEntryFlags::AP_HIGHER;
        }

        if value.contains(PageEntryFlags::USER_ACCESSIBLE) {
            flags |= ArchEntryFlags::AP_LOWER;
        }
        flags
    }
}

#[inline(always)]
const fn l0_index(addr: usize) -> usize {
    (addr >> 39) & 0x1FF
}

#[inline(always)]
const fn l1_index(addr: usize) -> usize {
    (addr >> 30) & 0x1FF
}
#[inline(always)]
const fn l2_index(addr: usize) -> usize {
    (addr >> 21) & 0x1FF
}
#[inline(always)]
const fn l3_index(addr: usize) -> usize {
    (addr >> 12) & 0x1FF
}

/// translates a
fn translate(addr: VirtAddr) -> (bool, usize, usize, usize, usize) {
    let addr = addr.into_raw();
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
                PhysAddr::from((self.0 & (((1 << 47) - 1) << 12)) as usize),
            ));
        }
        None
    }

    const fn new(flags: ArchEntryFlags, addr: PhysAddr) -> Self {
        Self(addr.into_raw() as u64 | flags.bits())
    }

    const fn set(&mut self, flags: ArchEntryFlags, addr: PhysAddr) {
        *self = Self::new(flags, addr)
    }

    #[inline(always)]
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    fn map(&mut self) -> Result<&'static mut ArchPageTable, MapToError> {
        let flags = ArchEntryFlags::TABLE_DESC | ArchEntryFlags::PRESENT;
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
            self.set(ArchEntryFlags::empty(), PhysAddr::null());
            frame_allocator::deallocate_frame(frame);
        }
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
#[repr(align(0x1000))]
pub struct ArchPageTable([Entry; 512]);

impl Index<usize> for ArchPageTable {
    type Output = Entry;
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for ArchPageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

/// Returns the current higher half root table
pub unsafe fn current_higher_root_table() -> FramePtr<ArchPageTable> {
    let ttbr1_el1: usize;
    unsafe {
        asm!("mrs {}, ttbr1_el1", out(reg) ttbr1_el1);
        let frame = Frame::containing_address(PhysAddr::from(ttbr1_el1));
        frame.into_ptr()
    }
}

/// Returns the current lower half root table
pub unsafe fn current_lower_root_table() -> FramePtr<ArchPageTable> {
    let ttbr0_el1: usize;
    unsafe {
        asm!("mrs {}, ttbr0_el1", out(reg) ttbr0_el1);
        let frame = Frame::containing_address(PhysAddr::from(ttbr0_el1));
        frame.into_ptr()
    }
}

/// FIXME: We use this when booting other CPUs for now, there is likely a better solution
pub(super) static CURRENT_HIGHER_HALF_TABLE: SyncUnsafeCell<PhysAddr> =
    SyncUnsafeCell::new(PhysAddr::null());

/// Sets the physical address of `ttbr1_el1` to `phys_addr`
pub(super) unsafe fn set_current_higher_page_table_phys(phys_addr: PhysAddr) {
    unsafe {
        asm!("msr ttbr1_el1, {}", in(reg) phys_addr.into_raw());
        let mair = SYS_MAIR;
        mair.sync();
        // reload address space
        asm!(
            "
            dsb ish
            tlbi VMALLE1
            dsb ish
            isb
            "
        );
    }
}
/// Sets the current higher half Page Table to `page_table`
pub unsafe fn set_current_higher_page_table(page_table: FramePtr<ArchPageTable>) {
    let ttbr1_el1: PhysAddr = page_table.phys_addr();
    unsafe {
        set_current_higher_page_table_phys(ttbr1_el1);
        *CURRENT_HIGHER_HALF_TABLE.get() = ttbr1_el1
    }
}

// TODO: maybe use traits here
impl ArchPageTable {
    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    unsafe fn free(&mut self, level: u8) {
        unsafe {
            core::arch::asm!("dsb ish; isb sy");

            for entry in &mut self.0 {
                if entry.flags().contains(ArchEntryFlags::PRESENT) {
                    entry.free(level - 1);
                }
            }
        }
    }

    #[inline]
    fn map_to_single(
        &mut self,
        page: Page,
        frame: Frame,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.addr());
        let flags: ArchEntryFlags = flags.into();
        let l1 = self[l0_index].map()?;
        let l2 = l1[l1_index].map()?;
        let l3 = l2[l2_index].map()?;
        let entry = &mut l3[l3_index];

        if entry.frame().is_some() {
            return Err(MapToError::AlreadyMapped);
        }

        entry.set(flags, frame.start_address());
        Ok(())
    }

    #[inline]
    /// Get a mutable reference to the entry for a given page
    unsafe fn get_entry(&self, page: Page) -> Option<&mut Entry> {
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.addr());
        let l1 = self[l0_index].mapped_to()?;
        let l2 = l1[l1_index].mapped_to()?;
        let l3 = l2[l2_index].mapped_to()?;

        Some(&mut l3[l3_index])
    }

    unsafe fn unmap_single(&mut self, page: Page) -> Option<Frame> {
        let entry = unsafe { self.get_entry(page)? };
        let frame = entry.frame();
        entry.set(ArchEntryFlags::empty(), PhysAddr::null());
        frame
    }
}

impl PageTableOps for ArchPageTable {
    unsafe fn deallocate(&mut self) {
        unsafe { self.free(4) }
    }

    unsafe fn zeroize(&mut self) {
        *self = unsafe { core::mem::zeroed() };
    }

    fn get_frame_of(&self, page: Page) -> Option<Frame> {
        unsafe { self.get_entry(page)?.frame() }
    }

    fn map_range(
        &mut self,
        pages: crate::memory::paging::IterPage,
        frames: frame_allocator::FrameIter,
        flags: crate::memory::paging::PageEntryFlags,
    ) -> Result<(), MapToError> {
        for (page, frame) in pages.zip(frames) {
            self.map_to_single(page, frame, flags)?;
        }

        Ok(())
    }
    // Higher half table and lower half's table are different
    unsafe fn sync_higher_half(&mut self) {}

    unsafe fn unmap_range<F>(
        &mut self,
        pages: crate::memory::paging::IterPage,
        mut with_each: F,
    ) -> Result<(), MapToError>
    where
        F: FnMut(Page, Frame),
    {
        for page in pages {
            let frame = unsafe { self.unmap_single(page) }.ok_or(MapToError::NotMapped)?;
            with_each(page, frame);
        }

        Ok(())
    }

    fn finish_unmap_ops(&mut self, pages: crate::memory::paging::IterPage) {
        tlb::flush_cache_range(pages.current_addr(), pages.end_addr())
    }
}

/// Maps architecture specific devices such as the UART serial in aarch64
pub unsafe fn map_devices(vmm: &mut VirtualMemoryManager) -> Result<(), MapToError> {
    unsafe {
        super::serial::map_serial(vmm);
        Ok(())
    }
}
