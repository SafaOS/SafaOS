use core::alloc::{GlobalAlloc, Layout};

use crate::{
    debug,
    memory::paging::MapToError,
    utils::locks::{LazyLock, Mutex},
};

use super::{
    VirtAddr, align_up,
    paging::{EntryFlags, current_higher_root_table},
};

pub const INIT_HEAP_SIZE: usize = (1024 * 1024) / 2;

#[derive(Debug, Clone)]
pub struct Block {
    free: bool,
    size: usize,
}

impl Block {
    #[inline]
    /// unsafe because there may be no next block causing UB
    /// use BuddyAllocator::next instead
    pub unsafe fn next<'b>(&self) -> &'b mut Block {
        unsafe {
            let end = (self as *const Self).byte_add(self.size);
            &mut *end.cast_mut()
        }
    }

    pub unsafe fn data(&mut self) -> *mut u8 {
        unsafe { (self as *mut Self).offset(1).cast() }
    }
    /// divides self into 2 buddies
    /// returns the right buddy
    /// self is still valid and it points to the left buddy
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
    heap_end: VirtAddr,
}

const fn align_to_power_of_2(size: usize) -> usize {
    let mut results = 1;
    while size > results {
        results <<= 1;
    }
    results
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
    align_to_power_of_2(size + size_of::<Block>())
}

impl BuddyAllocator<'_> {
    /// unsafe because size has to be a power of 2, has to contain Block header size and
    /// self.heap_end .. self.heap_end + size shall be mapped and not used by anything
    /// adds a free block with size `size` to the end of the allocator
    pub unsafe fn add_free<'b>(&mut self, size: usize) -> &'b mut Block {
        let new_block = self.heap_end.into_ptr::<Block>();
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
        let actual_end = unsafe {
            let start = self.heap_end;
            let end = start + size;

            current_higher_root_table()
                .alloc_map(start, end, EntryFlags::WRITE)
                .ok()?
        };

        debug!(
            BuddyAllocator,
            "expandition done end is at: {:#x}..{:#x} ...", self.heap_end, actual_end
        );

        let size = actual_end - self.heap_end;
        unsafe { Some(self.add_free(size)) }
    }

    pub fn create() -> Result<Self, MapToError> {
        let (possible_start, _) = super::sorcery::HEAP;

        let start = align_up(possible_start.into_raw(), size_of::<Block>());
        let start = align_up(start, 2);
        let start = VirtAddr::from(start);

        let diff = start.into_raw() - possible_start.into_raw();
        let size = align_down_to_power_of_2(INIT_HEAP_SIZE - diff);
        let end = start + size;

        let flags = EntryFlags::WRITE;
        let mut root_table = unsafe { current_higher_root_table() };
        unsafe {
            root_table.alloc_map(start, end, flags)?;
        }

        debug!(
            BuddyAllocator,
            "initing at {:#x}..{:#x} instead of {:#x} with size: {:#x}",
            start,
            end,
            possible_start,
            size
        );

        unsafe {
            let head = &mut *(start.into_ptr::<Block>());
            head.free = true;
            head.size = size;

            debug!(BuddyAllocator, "inited ...");
            Ok(Self {
                head: &mut *(head as *mut Block),
                tail: head,
                heap_end: end,
            })
        }
    }

    #[inline]
    /// safe wrapper around Block::next
    pub fn next<'b>(heap_end: VirtAddr, block: &Block) -> Option<&'b mut Block> {
        if VirtAddr::from(block as *const _ as usize + block.size) >= heap_end {
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

    /// coalescence buddies returns whether or not it coalescenced anything
    /// doesn't perform full coalescence
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

    /// performs full coalescence_buddies
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
        } else {
            if self.expand_heap_by(size).is_none() {
                return core::ptr::null_mut();
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

unsafe impl GlobalAlloc for LazyLock<Mutex<BuddyAllocator<'static>>> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock().allocmut(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            _ = layout;
            self.lock().deallocmut(ptr);
        }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: LazyLock<Mutex<BuddyAllocator>> = LazyLock::new(|| {
    Mutex::new(BuddyAllocator::create().expect("Failed to create buddy allocator"))
});

#[test_case]
fn buddy_allocator_test() {
    use alloc::vec::Vec;

    let mut test = Vec::new();

    for i in 0..100 {
        test.push(i);
    }

    crate::println!("{:#?}\nAllocated Vec with len {}", test, test.len());
}
