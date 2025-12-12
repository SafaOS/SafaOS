use core::{hint::unreachable_unchecked, ptr::NonNull};

use crate::{
    VirtAddr,
    memory::{
        AlignToPage,
        frame_allocator::{self, FramePtr},
        paging::PAGE_SIZE,
        vmm::VMMMFlags,
    },
};

#[derive(Debug)]
enum ObjectsEntry {
    Empty,
    Taken(VMMObject),
}

const EXTRA_PAGE_BYTES: usize = size_of::<u128>() /* bitmap */ + size_of::<Option<FramePtr<VMMObjectsPage>>>() /* next */;
const MAX_OBJECTS_COUNT: usize = (PAGE_SIZE - EXTRA_PAGE_BYTES) / size_of::<ObjectsEntry>();

#[derive(Debug)]
pub struct VMMObjectsPage {
    /// Bitmap of taken allocated objects in this page.
    bitmap: u128,
    /// Unordered set of objects in this page.
    ///
    /// This allows for fast insertion, and removal because we don't need to maintain order,
    /// instead the order is maintained by the objects themselves in a linked list.
    objects: [ObjectsEntry; MAX_OBJECTS_COUNT],
    pub next: Option<FramePtr<VMMObjectsPage>>,
}

const _: () = assert!(size_of::<VMMObjectsPage>() <= PAGE_SIZE);

impl VMMObjectsPage {
    const fn new() -> Self {
        Self {
            bitmap: 0,
            objects: [const { ObjectsEntry::Empty }; MAX_OBJECTS_COUNT],
            next: None,
        }
    }

    pub fn allocate() -> Result<FramePtr<Self>, ()> {
        frame_allocator::allocate_frame()
            .map(|frame| {
                let mut ptr = unsafe { frame.into_ptr::<Self>() };
                *ptr = Self::new();
                ptr
            })
            .ok_or(())
    }

    fn index_of(&self, obj: *const VMMObject) -> usize {
        unsafe { (obj).offset_from(self.objects.as_ptr() as *const VMMObject) as usize }
    }

    fn remove_object(&mut self, obj: *const VMMObject) -> VMMObject {
        let index = self.index_of(obj);
        match core::mem::replace(&mut self.objects[index], ObjectsEntry::Empty) {
            ObjectsEntry::Taken(obj) => {
                self.bitmap &= !(1 << index);
                obj
            }
            ObjectsEntry::Empty => panic!("Attempted to remove an object that was not present"),
        }
    }

    pub fn add_object(&mut self, obj: VMMObject) -> Result<NonNull<VMMObject>, ()> {
        // find a free index in the bitmap
        let index = self.bitmap.trailing_ones() as usize;
        if index >= self.objects.len() {
            // Try to push to sister objects page
            return if let Some(ref mut next) = self.next {
                next.add_object(obj)
            } else {
                let next = Self::allocate()?;
                self.next.insert(next).add_object(obj)
            };
        }

        self.bitmap |= 1 << index;
        self.objects[index] = ObjectsEntry::Taken(obj);
        match &self.objects[index] {
            ObjectsEntry::Taken(obj) => Ok(NonNull::from_ref(obj)),
            ObjectsEntry::Empty => unsafe { unreachable_unchecked() },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ObjectState {
    Free,
    #[allow(unused)]
    Allocated(VMMMFlags),
    #[allow(unused)]
    DMAAllocated(VMMMFlags),
    #[allow(unused)]
    LazyAllocated(VMMMFlags),
}

#[derive(Debug, Clone, Copy)]
pub struct VMMObject {
    addr: VirtAddr,
    pub name: &'static &'static str,
    pub state: ObjectState,
    /// Size of the region in bytes.
    ///
    /// although we always want to use pages.
    size: usize,

    next: Option<NonNull<VMMObject>>,
    prev: Option<NonNull<VMMObject>>,
}

impl VMMObject {
    pub fn addr(&self) -> VirtAddr {
        self.addr
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn prev_mut(&mut self) -> Option<&mut VMMObject> {
        self.prev.map(|mut ptr| unsafe { ptr.as_mut() })
    }

    pub fn prev(&self) -> Option<&VMMObject> {
        self.prev.map(|ptr| unsafe { ptr.as_ref() })
    }

    pub fn next_mut(&mut self) -> Option<&mut VMMObject> {
        self.next.map(|mut ptr| unsafe { ptr.as_mut() })
    }

    pub fn next(&self) -> Option<&VMMObject> {
        self.next.map(|ptr| unsafe { ptr.as_ref() })
    }

    #[inline(always)]
    pub const fn as_non_null(&self) -> NonNull<Self> {
        NonNull::from_ref(self)
    }

    pub fn insert_next(&mut self, mut new_next: NonNull<VMMObject>) {
        let new_next_ref = unsafe { new_next.as_mut() };
        let old_next = self.next.replace(new_next);

        new_next_ref.next = old_next;
        if let Some(mut old_next) = old_next {
            unsafe { old_next.as_mut().prev = Some(new_next) };
        }

        new_next_ref.prev = Some(NonNull::from_mut(self));
    }

    pub fn insert_prev(&mut self, mut new_prev: NonNull<VMMObject>) {
        let new_prev_ref = unsafe { new_prev.as_mut() };
        let old_prev = self.prev.replace(new_prev);

        new_prev_ref.prev = old_prev;
        if let Some(mut old_prev) = old_prev {
            unsafe { old_prev.as_mut().next = Some(new_prev) };
        }

        new_prev_ref.next = Some(self.as_non_null());
    }

    /// Attempts to absorb the next object into this one.
    ///
    /// returns a ptr to the new object if successful, otherwise and if there is no next None.
    pub fn try_absorb_right(&mut self) -> (Option<NonNull<Self>>, bool) {
        if let Some(ref right_ptr) = self.next {
            let right = {
                if unsafe { right_ptr.as_ref().allocated() } {
                    return (None, false);
                }
                unsafe { Self::remove_in_place(*right_ptr) }
            };

            self.size += right.size;
            self.next = right.next;

            if let Some(mut next) = self.next {
                unsafe { next.as_mut().prev = Some(self.as_non_null()) };
            }

            (self.next, true)
        } else {
            (None, false)
        }
    }

    /// Attempts to absorb the previous object into this one.
    ///
    /// returns a ptr to the new object if successful, otherwise and if there is no previous None.
    pub fn try_absorb_left(&mut self) -> (Option<NonNull<Self>>, bool) {
        if let Some(ref left_ptr) = self.prev {
            let left = {
                if unsafe { left_ptr.as_ref().allocated() } {
                    return (None, false);
                }
                unsafe { Self::remove_in_place(*left_ptr) }
            };

            self.size += left.size;
            self.addr = left.addr;
            self.prev = left.prev;

            if let Some(mut prev) = self.prev {
                unsafe { prev.as_mut().next = Some(self.as_non_null()) };
            }
            (self.prev, true)
        } else {
            (None, false)
        }
    }

    #[inline(always)]
    fn objects_page(&self) -> NonNull<VMMObjectsPage> {
        unsafe {
            NonNull::new_unchecked(
                ((self as *const VMMObject) as usize).to_previous_page() as *mut VMMObjectsPage
            )
        }
    }

    fn objects_page_mut(&mut self) -> &mut VMMObjectsPage {
        unsafe { self.objects_page().as_mut() }
    }

    /// Deallocates the given `VMMObject`.
    ///
    /// Safety: `ptr` must be a valid pointer to a `VMMObject`, it becomes invalid after this call.
    pub unsafe fn remove_in_place(mut ptr: NonNull<Self>) -> VMMObject {
        let this = unsafe {
            ptr.as_mut()
                .objects_page()
                .as_mut()
                .remove_object(ptr.as_ptr())
        };
        this
    }

    #[inline]
    pub const fn allocated(&self) -> bool {
        matches!(
            self.state,
            ObjectState::Allocated(_)
                | ObjectState::DMAAllocated(_)
                | ObjectState::LazyAllocated(_)
        )
    }

    pub const fn new_free(addr: VirtAddr, size: usize) -> Self {
        Self {
            name: &"UNNAMED",
            addr,
            state: ObjectState::Free,
            size,
            next: None,
            prev: None,
        }
    }

    #[inline(always)]
    pub const fn region_end(&self) -> VirtAddr {
        self.addr + self.size
    }

    /// Spilt the region so that it starts at self.region_start + `offset` and has `size` bytes.
    ///
    /// Returns a tuple of the newly created left and right objects, if they exist.
    pub fn split_at(
        &mut self,
        offset: usize,
        size: usize,
    ) -> Result<(Option<NonNull<Self>>, Option<NonNull<Self>>), ()> {
        if offset == 0 {
            let results = self.split_to_fit(size)?;
            return Ok((None, results));
        }

        // Spilt into 3 regions
        // - left
        // - this
        // - right
        let left_obj = VMMObject::new_free(self.addr, offset);

        let right_addr = self.addr + offset + size;
        let right_size = self.size - size - offset;
        let right_obj = (right_size != 0).then(|| VMMObject::new_free(right_addr, right_size));

        // Allocate left and right
        let left_ptr = self.objects_page_mut().add_object(left_obj)?;
        let right_ptr = match right_obj.map(|obj| self.objects_page_mut().add_object(obj)) {
            None => None,
            Some(Ok(ptr)) => Some(ptr),
            Some(Err(_)) => {
                unsafe { Self::remove_in_place(left_ptr) };
                return Err(());
            }
        };

        self.addr += offset;
        self.size = size;
        self.insert_prev(left_ptr);
        if let Some(right) = right_ptr {
            self.insert_next(right);
        }
        Ok((Some(left_ptr), right_ptr))
    }

    /// Splits the region so it has `fit` size of bytes, the rest is put into a new object, if that object was created and inserted a pointer to it will be returned.
    pub fn split_to_fit(&mut self, fit: usize) -> Result<Option<NonNull<Self>>, ()> {
        // Split the region if it's larger than the requested size
        if self.size > fit {
            let rest_size = self.size - fit;
            let rest_addr = self.addr + fit;

            let rest_meta = VMMObject::new_free(rest_addr, rest_size);

            self.size -= rest_size;
            let rest_ptr = self.objects_page_mut().add_object(rest_meta)?;
            self.insert_next(rest_ptr);
            Ok(Some(rest_ptr))
        } else {
            Ok(None)
        }
    }
}
