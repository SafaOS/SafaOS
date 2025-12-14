use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};

use alloc::vec::Vec;

use crate::{
    debug,
    memory::{AlignTo, AlignToPage, vmm::VMMAlloc},
    utils::locks::{LazyLock, Mutex},
};

use super::VirtAddr;

pub const INIT_HEAP_SIZE: usize = (1024 * 1024) / 2;

#[derive(Debug, Clone)]
pub struct Block {
    free: bool,
    size: usize,
}

impl Block {
    fn as_non_null(&self) -> NonNull<Self> {
        NonNull::from_ref(self)
    }

    #[inline]
    /// unsafe because there may be no next block causing UB
    /// use BuddyAllocator::next instead
    pub unsafe fn next_non_null(&self) -> NonNull<Self> {
        unsafe { self.as_non_null().byte_add(self.size) }
    }

    pub fn data(&mut self) -> NonNull<[u8]> {
        let ptr: NonNull<u8> = unsafe { (NonNull::from_mut(self)).offset(1).cast() };
        NonNull::slice_from_raw_parts(ptr, self.size - size_of::<Block>())
    }
    /// divides self into 2 buddies
    /// returns the right buddy
    /// self is still valid and it points to the left buddy
    /// both self and buddy is free after this
    pub fn divide<'b>(&mut self) -> NonNull<Block> {
        self.free = true;
        self.size >>= 1;

        let buddy = unsafe { &mut *(self as *mut Self).byte_add(self.size) };
        buddy.free = true;
        buddy.size = self.size;

        NonNull::from_mut(buddy)
    }

    /// divides self until it's size is `size`
    /// returns the right most buddy
    /// returns None if it is already fit
    pub fn spilt_to_fit<'b>(&mut self, size: usize) -> Option<NonNull<Block>> {
        let mut buddy = None;

        while (self.size / 2) >= size && (self.size / 2) > size_of::<Block>() {
            buddy = Some(self.divide());
        }

        buddy
    }
}

#[derive(Debug)]
pub struct BuddyAllocator {
    tail: NonNull<Block>,
    heap: Vec<u8, VMMAlloc>,
}

const fn align_down_to_power_of_2(size: usize) -> usize {
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
    (size + size_of::<Block>()).next_power_of_two()
}

impl BuddyAllocator {
    fn heap_start(&self) -> VirtAddr {
        VirtAddr::from_ptr(self.heap.as_ptr())
    }

    fn head(&self) -> NonNull<Block> {
        unsafe { NonNull::new_unchecked(self.heap.as_ptr().cast_mut().cast::<Block>()) }
    }

    fn heap_end(&self) -> VirtAddr {
        unsafe { VirtAddr::from_ptr(self.heap.as_ptr().add(self.heap.len())) }
    }
    /// unsafe because size has to be a power of 2, has to contain Block header size and
    /// self.heap_end .. self.heap_end + size shall be mapped and not used by anything
    /// adds a free block with size `size` to the end of the allocator
    pub unsafe fn reserve_free<'b>(&mut self, size: usize) -> NonNull<Block> {
        debug!(BuddyAllocator, "expanding the heap by {:#x}", size);

        self.heap.reserve(size);
        let new_block = self.heap_end().into_ptr::<Block>();
        unsafe {
            self.heap.set_len(self.heap.len() + size);
        }

        unsafe {
            (*new_block).free = true;
            (*new_block).size = size;

            let ptr = NonNull::new_unchecked(new_block);
            self.tail = ptr;
            ptr
        }
    }

    pub fn expand_heap_by<'b>(&mut self, size: usize) -> Option<NonNull<Block>> {
        let size = size
            .max(self.heap.len())
            .to_next_page()
            .next_power_of_two()
            .to_next_multiple_of(size_of::<Block>());

        let results = unsafe { self.reserve_free(size) };
        debug!(
            BuddyAllocator,
            "expandition done end is at: {:#x}...",
            self.heap_end(),
        );
        Some(results)
    }

    pub fn create() -> Self {
        let hint = super::init::heap0_hint()
            .to_next_multiple_of(size_of::<Block>())
            .to_next_multiple_of(2);
        let size = align_down_to_power_of_2(INIT_HEAP_SIZE);

        let allocator = VMMAlloc::new(&"BuddyAllocator").with_hint(hint);
        let heap = Vec::with_capacity_in(size, allocator);
        let mut this = Self {
            tail: NonNull::dangling(),
            heap,
        };

        unsafe { this.reserve_free(size) };
        debug!(
            BuddyAllocator,
            "initing at {:#x}..{:#x} with size: {:#x}",
            this.heap_start(),
            this.heap_end(),
            size
        );

        debug!(BuddyAllocator, "inited ...");
        this
    }

    #[inline]
    /// safe wrapper around Block::next
    pub fn next(&self, block: NonNull<Block>) -> Option<NonNull<Block>> {
        let heap_end = self.heap_end();

        if VirtAddr::from(block.as_ptr() as usize + unsafe { block.as_ref().size }) >= heap_end {
            None
        } else {
            Some(unsafe { block.as_ref().next_non_null() })
        }
    }

    /// same as `spilt_to_fit_same` on `block`, however it also sets tail if the block was the previous
    /// tail
    pub fn spilt_to_fit(&mut self, mut block: NonNull<Block>, size: usize) -> NonNull<Block> {
        if let Some(used) = unsafe { block.as_mut().spilt_to_fit(size) } {
            if core::ptr::eq(block.as_ptr(), self.tail.as_ptr()) {
                self.tail = used;
            }

            used
        } else {
            block
        }
    }

    pub fn find_free_block<'b>(&mut self, size: usize) -> Option<NonNull<Block>> {
        let mut current = self.head();
        let mut best_block: Option<NonNull<Block>> = None;

        let Some(mut buddy) = self.next(current) else {
            return Some(self.spilt_to_fit(current, size));
        };

        loop {
            let block_r = unsafe { current.as_ref() };
            let buddy_r = unsafe { buddy.as_ref() };

            if block_r.free
                && block_r.size >= size
                && best_block.is_none_or(|x| unsafe { (x.as_ref()).size >= block_r.size })
            {
                best_block = Some(current);
            }

            if buddy_r.free
                && buddy_r.size >= size
                && best_block.is_none_or(|x| unsafe { x.as_ref().size >= buddy_r.size })
            {
                best_block = Some(buddy);
            }

            current = buddy;
            let Some(next_buddy) = self.next(current) else {
                break;
            };
            buddy = next_buddy;
        }

        let results = best_block?;
        self.spilt_to_fit(results, size);
        Some(results)
    }

    /// coalescence buddies returns whether or not it coalescenced anything
    /// doesn't perform full coalescence
    fn coalescence_buddies(&mut self) -> bool {
        let mut results = false;

        let mut block = self.head();
        let Some(mut buddy) = self.next(block) else {
            return false;
        };

        loop {
            let block_r = unsafe { block.as_mut() };
            let buddy_r = unsafe { buddy.as_ref() };

            if block_r.free && buddy_r.free && block_r.size == buddy_r.size {
                block_r.size <<= 1;
                results = true;
            } else {
                block = buddy;
            }

            let Some(next_buddy) = self.next(block) else {
                return results;
            };
            buddy = next_buddy;
        }
    }

    /// performs full coalescence_buddies
    fn coalescence_buddies_full(&mut self) {
        while self.coalescence_buddies() {}
    }

    pub fn allocmut(&mut self, layout: Layout) -> Option<NonNull<[u8]>> {
        let size = actual_size(layout.size());

        let block = if let Some(block) = self.find_free_block(size) {
            Some(block)
        } else {
            self.coalescence_buddies_full();
            self.find_free_block(size)
        };

        if let Some(mut block) = block {
            let block_mut = unsafe { block.as_mut() };
            block_mut.free = false;
            return Some(block_mut.data());
        } else {
            if self.expand_heap_by(size).is_none() {
                return None;
            };

            self.allocmut(layout)
        }
    }
    /// unsafe because ptr had to be allocated using self
    pub unsafe fn deallocmut(&mut self, ptr: *mut u8) {
        unsafe {
            let block: *mut Block = ptr.byte_sub(size_of::<Block>()).cast();
            (*block).free = true;
            self.coalescence_buddies_full();
        }
    }
}

unsafe impl GlobalAlloc for LazyLock<Mutex<BuddyAllocator>> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock()
            .allocmut(layout)
            .map(|s| s.as_ptr() as *mut u8)
            .unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            _ = layout;
            self.lock().deallocmut(ptr);
        }
    }
}

unsafe impl Sync for BuddyAllocator {}
unsafe impl Send for BuddyAllocator {}

#[global_allocator]
static GLOBAL_ALLOCATOR: LazyLock<Mutex<BuddyAllocator>> =
    LazyLock::new(|| Mutex::new(BuddyAllocator::create()));

#[test_case]
fn buddy_allocator_test() {
    use alloc::vec::Vec;

    let mut test = Vec::new();

    for i in 0..100 {
        test.push(i);
    }

    crate::println!("{:#?}\nAllocated Vec with len {}", test, test.len());
}
