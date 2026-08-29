use crate::arch::aarch64::registers::{DEVICE_UNCACHEABLE_MAIR_IDX, FRAMEBUFFER_CACHED_MAIR_IDX};
use crate::arch::aarch64::tlb;
use crate::memory::{frame_allocator, phys_to_virt};
use crate::misc::{Frame, FrameIter, IterPage, PAGE_SIZE, Page, PhysAddr, VirtAddr};
use crate::paging::{MapToError, PageEntryFlags, PageTableOps};
use core::ops::{Index, IndexMut};
use core::ptr::NonNull;

use bitfield_struct::bitfield;

#[derive(Debug, Clone, Copy)]
enum EntryShareability {
    None = 0b00,
    Outer = 0b10,
    Inner = 0b11,
}

impl EntryShareability {
    #[inline(always)]
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0b00 => Self::None,
            0b10 => Self::Outer,
            0b11 => Self::Inner,
            _ => Self::None,
        }
    }

    #[inline(always)]
    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

#[bitfield(u64)]
pub struct Entry {
    present: bool,
    table_desc: bool,
    /// Index into mair.
    #[bits(3)]
    mair: u8,
    non_secure: bool,
    /// If set user has access as described by [`Self::ap_higher`] otherwise no access for user.
    ap_lower: bool,
    /// If set it is read/only otherwise it is read/write.
    ap_higher: bool,
    #[bits(2)]
    shareability: EntryShareability,
    /// Fault on first access if false.
    access_flag: bool,
    not_global: bool,
    #[bits(40)]
    pagenum: u64,
    contiguous_hint: bool,
    /// Privileged Execute Never (EL1 cannot execute)
    pxn: bool,
    /// Unprivileged Execute Never (EL0 cannot execute)
    uxn: bool,
    #[bits(9)]
    _res: (),
}

impl From<PageEntryFlags> for Entry {
    fn from(value: PageEntryFlags) -> Self {
        let mut mair_index = 0;
        let mut flags = Entry::new()
            .with_present(true)
            .with_table_desc(true)
            .with_access_flag(true)
            .with_shareability(EntryShareability::Inner);

        if value.contains(PageEntryFlags::DEVICE_UNCACHEABLE) {
            mair_index = DEVICE_UNCACHEABLE_MAIR_IDX;
        } else if value.contains(PageEntryFlags::FRAMEBUFFER_CACHED) {
            mair_index = FRAMEBUFFER_CACHED_MAIR_IDX;
        }

        flags.set_mair(mair_index);

        if !value.contains(PageEntryFlags::WRITE) {
            // read-only flag
            flags.set_ap_higher(true);
        }

        if value.contains(PageEntryFlags::DISABLE_EXEC) {
            flags.set_pxn(true);
            flags.set_uxn(true);
        }

        // Always not allow kernel/user from executing user/kernel code.
        if value.contains(PageEntryFlags::USER_ACCESSIBLE) {
            flags.set_ap_lower(true);
            // kernel cannot execute.
            flags.set_pxn(true);
        } else {
            // user cannot execute.
            flags.set_uxn(true);
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
    let addr = addr.raw();
    let is_higher_half = (addr >> 63) & 1 == 1;
    (
        is_higher_half,
        l0_index(addr),
        l1_index(addr),
        l2_index(addr),
        l3_index(addr),
    )
}

impl Entry {
    fn addr(&self) -> Option<PhysAddr> {
        if self.present() {
            return Some(PhysAddr::new(self.pagenum() as usize * PAGE_SIZE));
        }
        None
    }

    fn frame(&self) -> Option<Frame> {
        self.addr().map(|a| Frame::containing(a))
    }

    const fn set_addr(&mut self, addr: PhysAddr) {
        self.set_pagenum(addr.page_num() as u64);
    }

    const fn with_addr(mut self, addr: PhysAddr) -> Self {
        self.set_addr(addr);
        self
    }

    #[inline(always)]
    /// if the entry is not present it allocates a new frame and uses it's address as entry's
    /// then returns the entry address as a pagetable
    unsafe fn map(&mut self) -> Result<NonNull<ArchPageTable>, MapToError> {
        if let Some(phys_addr) = self.addr() {
            debug_assert!(
                self.table_desc(),
                "Should be both present and a table desc, we currently don't handle block descriptors at all"
            );

            let virt_addr = phys_to_virt(phys_addr);
            let entry_ptr = virt_addr.into_ptr::<ArchPageTable>();

            Ok(NonNull::new(entry_ptr)
                .expect("Failed to create a pointer to page table entry addr"))
        } else {
            let frame = frame_allocator::allocate_frame()?;
            let phys_addr = frame.addr();

            let virt_addr = phys_to_virt(phys_addr);
            let table_ptr = virt_addr.into_ptr::<ArchPageTable>();
            unsafe { (*table_ptr).zeroize() };

            // Regardless if this entry points to a table or not we want both to be true, because a level 3 entry that isn't a table desc is UB for some reason.
            self.set_present(true);
            self.set_table_desc(true);
            self.set_access_flag(true);
            self.set_addr(phys_addr);

            Ok(NonNull::new(table_ptr).unwrap())
        }
    }

    /// if an entry is mapped returns the PageTable or the Frame(as a PageTable) it is mapped to
    fn mapped_to(&self) -> Option<NonNull<ArchPageTable>> {
        if let Some(phys_addr) = self.addr() {
            let virt_addr = phys_to_virt(phys_addr);
            let entry_ptr = virt_addr.into_ptr::<ArchPageTable>();

            return Some(NonNull::new(entry_ptr).unwrap());
        }

        None
    }

    /// deallocates an entry depending on it's level if it is 1 it should just deallocate the frame
    /// otherwise treat the frame as a page table and deallocate it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    unsafe fn free(&mut self, level: u8) {
        unsafe {
            let addr = self.addr().unwrap();

            if level != 0 {
                let table = &mut *(phys_to_virt(addr).into_ptr::<ArchPageTable>());
                table.free(level);
            }
            self.deallocate();
        }
    }

    /// deallocates a page table entry and invalidates it
    /// # Safety
    /// the caller must ensure that the entry is not used anymore
    unsafe fn deallocate(&mut self) {
        if let Some(addr) = self.addr() {
            self.set_present(false);
            unsafe {
                frame_allocator::deallocate_frame(Frame::containing(addr))
                    .expect("Failed to deallocate an entry")
            };
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
pub fn current_kernel() -> NonNull<ArchPageTable> {
    let ttbr1_el1: usize;
    unsafe {
        core::arch::asm!("mrs {}, ttbr1_el1", out(reg) ttbr1_el1, options(nostack, preserves_flags, nomem));
        let addr = PhysAddr::from(ttbr1_el1);
        NonNull::new_unchecked(phys_to_virt(addr).into_ptr::<ArchPageTable>())
    }
}

/// Returns the current lower half root table
pub fn current_user() -> NonNull<ArchPageTable> {
    let ttbr0_el1: usize;
    unsafe {
        core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0_el1, options(nostack, preserves_flags, nomem));
        let addr = PhysAddr::from(ttbr0_el1);
        if *addr == 0 {
            panic!("user page table is null");
        }
        NonNull::new(phys_to_virt(addr).into_ptr::<ArchPageTable>())
            .expect("user page table is null")
    }
}

/// Sets the physical address of `ttbr1_el1` to `addr`
pub unsafe fn set_current_kernel(addr: PhysAddr) {
    unsafe {
        core::arch::asm!("msr ttbr1_el1, {}", in(reg) addr.raw());
        // Reload address space
        core::arch::asm!(
            "
            dsb ish
            tlbi VMALLE1
            dsb ish
            isb
            "
        );
    }
}

impl ArchPageTable {
    /// deallocates a page table including it's entries, doesn't deallocate the higher half!
    unsafe fn free(&mut self, level: u8) {
        unsafe {
            // Ensure all reads/writes are completed before completely ending this.
            core::arch::asm!("dsb ish; isb sy");

            for entry in &mut self.0 {
                if entry.present() {
                    entry.free(level - 1);
                }
            }
        }
    }

    #[inline]
    /// Safety: All access to a page table is inheirtly unsafe
    /// after use a dsb ish barrier must be executed to ensure all the writes are done first.
    unsafe fn map_to_single(
        &mut self,
        page: Page,
        frame: Frame,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.addr());
        let mapped_entry: Entry = flags.into();
        let mapped_entry = mapped_entry.with_addr(frame.addr());

        unsafe {
            let mut l1 = self[l0_index].map()?;
            let mut l2 = l1.as_mut()[l1_index].map()?;
            let mut l3 = l2.as_mut()[l2_index].map()?;
            let entry = &mut l3.as_mut()[l3_index];

            if entry.addr().is_some() {
                return Err(MapToError::AlreadyMapped);
            }

            *entry = mapped_entry;
        }

        Ok(())
    }

    #[inline]
    /// Get a mutable reference to the entry for a given page
    unsafe fn get_entry(&self, page: Page) -> Option<&mut Entry> {
        let (_, l0_index, l1_index, l2_index, l3_index) = translate(page.addr());
        unsafe {
            let mut l1 = self[l0_index].mapped_to()?;
            let mut l2 = l1.as_mut()[l1_index].mapped_to()?;
            let mut l3 = l2.as_mut()[l2_index].mapped_to()?;

            Some(&mut l3.as_mut()[l3_index])
        }
    }

    unsafe fn unmap_single(&mut self, page: Page) -> Option<Frame> {
        let entry = unsafe { self.get_entry(page)? };
        let frame = entry.frame();
        entry.set_present(false);
        frame
    }
}

impl PageTableOps for ArchPageTable {
    unsafe fn deallocate(&mut self) {
        unsafe { self.free(4) }
    }

    unsafe fn zeroize(&mut self) {
        *self = unsafe { core::mem::zeroed() };
        unsafe { core::arch::asm!("dsb ishst; isb") };
    }

    fn get_frame_of(&self, page: Page) -> Option<Frame> {
        unsafe { self.get_entry(page)?.frame() }
    }

    fn map_range(
        &mut self,
        pages: IterPage,
        frames: FrameIter,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        for (page, frame) in pages.zip(frames) {
            unsafe { self.map_to_single(page, frame, flags) }?;
        }

        // Ensure writes are visible.
        // the entry was not mapped before so it is unneccassary to flush the TLB.
        unsafe { core::arch::asm!("dsb ish; isb") };
        Ok(())
    }
    // Higher half table and lower half's table are different
    unsafe fn sync_higher_half(&mut self) {}

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
            match unsafe { self.unmap_single(page) } {
                Some(frame) => {
                    with_each(page, frame);
                }
                None if lazy => {}
                None => return Err(MapToError::NotMapped),
            }
        }

        // Ensure writes are visible.
        tlb::flush_cache_range(pages.current_addr(), pages.end_addr());
        Ok(())
    }

    fn finish_ops(&mut self, _pages: crate::misc::IterPage) {
        // TLB invalidation is done in place by `unmap_range` and `set_flags`.
    }

    unsafe fn set_flags_range(
        &mut self,
        pages: IterPage,
        flags: PageEntryFlags,
    ) -> Result<(), MapToError> {
        let lazy = flags.contains(PageEntryFlags::_IS_LAZY);
        let empty_entry: Entry = flags.into();

        // break-before-make sequence for correctly modifying entries.
        // break:
        for page in pages {
            match unsafe { self.get_entry(page) } {
                Some(ent) if ent.addr().is_some() => ent.set_present(false),
                _ if lazy => {}
                _ => return Err(MapToError::NotMapped),
            }
        }

        // particularly an unmap.
        tlb::flush_cache_range(pages.current_addr(), pages.end_addr());

        // make
        //
        // a map
        for page in pages {
            match unsafe { self.get_entry(page) } {
                Some(ent) if ent.pagenum() != 0 => {
                    let f = PhysAddr::new(ent.pagenum() as usize * PAGE_SIZE);
                    *ent = empty_entry.with_addr(f);
                }
                _ if lazy => {}
                _ => unreachable!(
                    "Translation Table break-before-make sequence for `set_flags_range`, `make` didn't match `break` with Error: NotMapped"
                ),
            }
        }

        // Ensure the writes are visible.
        unsafe { core::arch::asm!("dsb ish; isb") };
        Ok(())
    }
}
