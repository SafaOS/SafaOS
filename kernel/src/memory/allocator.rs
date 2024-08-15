use core::{
    alloc::{GlobalAlloc, Layout},
    ptr,
};

use crate::{memory::align_up, utils::Locked};

#[derive(Debug)]
pub struct Node {
    size: usize,
    next: Option<&'static mut Node>,
}

impl Node {
    pub const fn new(size: usize) -> Self {
        Self { size, next: None }
    }

    pub fn start_addr(&self) -> usize {
        self as *const Self as usize
    }

    pub fn end_addr(&self) -> usize {
        self.start_addr() + self.size
    }

    /// checks if a node can hold `size` bytes aligned to `align_amount`
    pub fn can_hold(&self, size: usize, align_amount: usize) -> Result<usize, ()> {
        let start = align_up(self.start_addr(), align_amount);
        let end = start.checked_add(size).ok_or(())?;

        if end > self.end_addr() {
            return Err(());
        }

        let ecess_size = self.end_addr() - end;
        if ecess_size > 0 && ecess_size < size_of::<Node>() {
            // if we have an excess we check if we can use it for a new node or not if not Err
            return Err(());
        }

        Ok(start)
    }
}
#[derive(Debug)]
pub struct LinkedListAllocator {
    head: Node,
}

impl LinkedListAllocator {
    pub const fn new() -> Self {
        Self {
            head: Node {
                size: 0,
                next: None,
            },
        }
    }
    // heap_start has to be aligned
    pub unsafe fn init(&mut self, heap_start: usize, size: usize) {
        self.add_free_node(align_up(heap_start, align_of::<Node>()), size);
    }

    pub unsafe fn alloc_mut(&mut self, layout: Layout) -> *mut u8 {
        let (size, align) = Self::size_align(layout);

        if let Some((node, addr)) = self.find_free_node(size, align) {
            let alloc_end = addr.checked_add(size).expect("overflow");
            // divide block
            let excess_size = node.end_addr() - alloc_end;
            if excess_size > 0 {
                self.add_free_node(alloc_end, excess_size);
            }

            addr as *mut u8
        } else {
            ptr::null_mut()
        }
    }

    pub unsafe fn dealloc_mut(&mut self, ptr: *mut u8, layout: Layout) {
        let (size, _) = Self::size_align(layout);
        self.add_free_node(ptr as usize, size)
    }

    pub fn find_free_node(
        &mut self,
        size: usize,
        align: usize,
    ) -> Option<(&'static mut Node, usize)> {
        let mut current = &mut self.head;

        while let Some(ref mut node) = current.next {
            if let Ok(addr) = node.can_hold(size, align) {
                let next = node.next.take();
                let node = current.next.take().unwrap();

                current.next = next;

                return Some((node, addr));
            } else {
                current = current.next.as_mut().unwrap();
            }
        }

        // extanding the heap
        // let (page, mapper) = paging_mapper()
        //     .map_free_page_from(HEAP_START, EntryFlags::WRITABLE)
        //     .ok()?;
        //
        // unsafe {
        //     mapper.flush();
        //     self.add_free_node(page.start_address, PAGE_SIZE);
        // }
        // self.find_free_node(size, align)
        None
    }

    pub unsafe fn add_free_node(&mut self, addr: usize, size: usize) {
        assert_eq!(align_up(addr, align_of::<Node>()), addr);
        assert!(size >= size_of::<Node>());

        let mut node = Node::new(size);

        node.next = self.head.next.take();

        let node_ptr = addr as *mut Node;
        ptr::write(node_ptr, node);

        self.head.next = Some(&mut *node_ptr);
    }

    fn size_align(layout: Layout) -> (usize, usize) {
        let layout = layout
            .align_to(align_of::<Node>())
            .expect("adjusting alignment failed")
            .pad_to_align();

        let size = layout.size().max(size_of::<Node>());
        (size, layout.align())
    }
}

unsafe impl GlobalAlloc for Locked<LinkedListAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut allocator = self.inner.lock();
        allocator.alloc_mut(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut allocator = self.inner.lock();
        allocator.dealloc_mut(ptr, layout)
    }
}
