use core::{
    alloc::{GlobalAlloc, Layout},
    mem::MaybeUninit,
};

use crate::{debug, memory::frame_allocator, utils::Locked};

use super::{
    align_up,
    paging::{current_root_table, EntryFlags, IterPage, Page},
    VirtAddr,
};

#[derive(Debug, Clone)]
pub struct Block {
    free: bool,
    /// decreases header size
    /// we dont want more then 4gb of heap space anyways we want a few mbs
    size: usize,
}

impl Block {
    #[inline]
    /// unsafe because there may be no next block causing UB
    /// use BuddyAllocator::next instead
    pub unsafe fn next<'b>(&self) -> &'b mut Block {
        let end = (self as *const Self).byte_add(self.size);
        &mut *end.cast_mut()
    }

    pub unsafe fn data(&mut self) -> *mut u8 {
        (self as *mut Self).offset(1).cast()
    }
    /// divides self into 2 buddies
    /// returns the right buddy
    /// self is still vaild and it points to the left buddy
    /// both self and buddy is free after this
    pub fn divide<'b>(&mut self) -> &'b mut Block {
        self.free = true;
        self.size >>= 1;

        let buddy = unsafe { &mut *(self as *mut Self).byte_add(self.size) };
        buddy.free = true;
        buddy.size = self.size;

        buddy
    }

    /// divides self until it's size is `size`
    /// returns the right most buddy
    /// returns None if it is already fit
    pub fn spilt_to_fit<'b>(&mut self, size: usize) -> Option<&'b mut Block> {
        let mut buddy = None;

        while (self.size / 2) >= size && (self.size / 2) > size_of::<Block>() {
            buddy = Some(self.divide());
        }

        buddy
    }
}

#[derive(Debug)]
pub struct BuddyAllocator<'a> {
    head: &'a mut Block,
    tail: &'a mut Block,
    heap_end: usize,
}

fn align_to_power_of_2(size: usize) -> usize {
    let mut results = 1;
    while size > results {
        results <<= 1;
    }
    results
}

fn align_down_to_power_of_2(size: usize) -> usize {
    let mut results = 1;
    while size > results {
        results <<= 1;
    }

    if results != size {
        results >>= 1;
    }

    results
}

/// returns the actual block size, aligned to power of 2 including header size
fn actual_size(size: usize) -> usize {
    align_to_power_of_2(size + size_of::<Block>())
}

impl BuddyAllocator<'_> {
    pub const unsafe fn new() -> MaybeUninit<Self> {
        MaybeUninit::zeroed()
    }

    /// unsafe because size has to be a power of 2, has to contain Block header size and
    /// self.heap_end .. self.heap_end + size shall be mapped and not used by anything
    /// adds a free block with size `size` to the end of the allocator
    pub unsafe fn add_free<'b>(&mut self, size: usize) -> &'b mut Block {
        let new_block = self.heap_end as *mut Block;
        unsafe {
            (*new_block).free = true;
            (*new_block).size = size;

            self.tail = &mut *new_block;
            self.heap_end += size;
            &mut *new_block
        }
    }

    pub fn expand_heap_by<'b>(&mut self, size: usize) -> Option<&'b mut Block> {
        debug!(BuddyAllocator, "expanding the heap by {:#x}", size);
        let iter = IterPage {
            start: Page::containing_address(self.heap_end),
            end: Page::containing_address(self.heap_end + size),
        };

        for page in iter {
            unsafe {
                if current_root_table().get_frame(page).is_none() {
                    let frame = frame_allocator::allocate_frame()?;
                    current_root_table()
                        .map_to(page, frame, EntryFlags::PRESENT | EntryFlags::WRITABLE)
                        .ok()?;
                }
            }
        }

        debug!(BuddyAllocator, "expandition done ...");
        unsafe { Some(self.add_free(size)) }
    }

    pub unsafe fn init(&mut self, possible_start: VirtAddr, size: usize) {
        let start = align_up(possible_start, size_of::<Block>());
        let start = align_up(start, 2);

        let diff = start - possible_start;
        let size = align_down_to_power_of_2(size - diff);
        let end = start + size;

        debug!(
            BuddyAllocator,
            "initing at {:#x}..{:#x} instead of {:#x} with size: {:#x}",
            start,
            end,
            possible_start,
            size
        );

        let head = &mut *(start as *mut Block);
        head.free = true;
        head.size = size;

        self.head = &mut *(start as *mut Block);
        self.tail = head;
        self.heap_end = end;
    }

    #[inline]
    /// safe wrapper around Block::next
    pub fn next<'b>(heap_end: usize, block: &Block) -> Option<&'b mut Block> {
        if (block as *const _ as usize + block.size) >= heap_end {
            None
        } else {
            unsafe { Some(block.next()) }
        }
    }

    /// same as `spilt_to_fit_same` on `block`, however it also sets tail if the block was the previous
    /// tail
    pub fn spilt_to_fit<'b>(
        tail: &mut &mut Block,
        block: &mut Block,
        size: usize,
    ) -> &'b mut Block {
        if let Some(used) = block.spilt_to_fit(size) {
            if core::ptr::eq(block, *tail) {
                *tail = unsafe { &mut *(used as *mut _) };
            }

            used
        } else {
            unsafe { &mut *(block as *mut _) }
        }
    }

    pub fn find_free_block<'b>(&mut self, size: usize) -> Option<&'b mut Block> {
        let mut block = &mut *self.head;
        let mut best_block: Option<*mut Block> = None;

        let Some(mut buddy) = Self::next(self.heap_end, block) else {
            return Some(Self::spilt_to_fit(&mut self.tail, block, size));
        };

        loop {
            if block.free
                && block.size >= size
                && best_block.is_none_or(|x| unsafe { (*x).size >= block.size })
            {
                best_block = Some(block);
            }

            if buddy.free
                && buddy.size >= size
                && best_block.is_none_or(|x| unsafe { (*x).size >= buddy.size })
            {
                best_block = Some(buddy);
            }

            block = buddy;
            let Some(next_buddy) = Self::next(self.heap_end, block) else {
                break;
            };
            buddy = next_buddy;
        }

        let results = unsafe { &mut *best_block? };
        Self::spilt_to_fit(&mut self.tail, results, size);
        Some(results)
    }

    /// coalescence buddies returns wether or not it coalescenced anything
    /// doesn't peform full coalescence
    fn coalescence_buddies(&mut self) -> bool {
        let mut results = false;

        let mut block = &mut *self.head;
        let Some(mut buddy) = Self::next(self.heap_end, block) else {
            return false;
        };

        loop {
            if block.free && buddy.free && block.size == buddy.size {
                block.size <<= 1;
                results = true;
            } else {
                block = buddy;
            }

            let Some(next_buddy) = Self::next(self.heap_end, block) else {
                return results;
            };
            buddy = next_buddy;
        }
    }

    /// peforms full coalescence_buddies
    fn coalescence_buddies_full(&mut self) {
        while self.coalescence_buddies() {}
    }

    pub fn allocmut(&mut self, layout: Layout) -> *mut u8 {
        let size = actual_size(layout.size());

        let block = if let Some(block) = self.find_free_block(size) {
            Some(block)
        } else {
            self.coalescence_buddies_full();
            self.find_free_block(size)
        };

        if let Some(block) = block {
            block.free = false;
            return unsafe { block.data() };
        } else if let Some(block) = self.expand_heap_by(size) {
            block.free = false;
            return unsafe { block.data() };
        }

        core::ptr::null_mut()
    }
    /// unsafe because ptr had to be allocated using self
    pub unsafe fn deallocmut(&mut self, ptr: *mut u8) {
        let block: *mut Block = ptr.byte_sub(size_of::<Block>()).cast();
        (*block).free = true;
        self.coalescence_buddies_full();
    }
}

unsafe impl GlobalAlloc for Locked<MaybeUninit<BuddyAllocator<'static>>> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.inner.lock().assume_init_mut().allocmut(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        _ = layout;
        self.inner.lock().assume_init_mut().deallocmut(ptr);
    }
}
