// a pmm i believe

use core::slice;

use lazy_static::lazy_static;
use spin::Mutex;

use crate::debug;

use super::{align_down, align_up, paging::PAGE_SIZE, PhysAddr};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub start_address: PhysAddr,
}

impl Frame {
    #[inline]
    // returns the frame that contains an address
    pub fn containing_address(address: PhysAddr) -> Self {
        Self {
            start_address: align_down(address, PAGE_SIZE), // for now frames can only be 1 normal page sized
        }
    }
}

pub type Bitmap = &'static mut [u8];

#[derive(Debug)]
pub struct RegionAllocator {
    /// keeps track of which frame is used or not
    pub bitmap: Bitmap,
}

impl RegionAllocator {
    /// limine
    /// TODO: look at setting unsable frames as used in the bitmap
    pub fn new() -> Self {
        let mmap = crate::limine::mmap_request();
        // figuring out how much frames we have
        let mut last_usable_entry = None;
        let mut first_usable_entry = None;

        for entry in mmap.entries() {
            if entry.entry_type == limine::memory_map::EntryType::USABLE {
                if first_usable_entry.is_none() {
                    first_usable_entry = Some(entry);
                }
                last_usable_entry = Some(entry);
            }
        }

        let last_usable_entry = last_usable_entry.unwrap();

        let frame_count = align_down(
            (last_usable_entry.base + last_usable_entry.length) as usize,
            PAGE_SIZE,
        ) / PAGE_SIZE;

        debug!(
            RegionAllocator,
            "{} usable bytes found",
            frame_count * PAGE_SIZE
        );

        // frame_count is the number of bits
        // aligns to 8 to make sure we can get a vaild number of bytes for our frame
        let bytes = align_up(frame_count, 8) / 8;

        // finds a place the bitmap can live in
        let mut best_region: Option<&limine::memory_map::Entry> = None;

        for entry in mmap.entries() {
            if entry.entry_type == limine::memory_map::EntryType::USABLE
                && entry.length as usize >= bytes
                && (best_region.is_none() || best_region.is_some_and(|x| x.length > entry.length))
            {
                best_region = Some(entry);
            }
        }

        debug_assert!(best_region.is_some());

        let best_region = best_region.unwrap();
        let bitmap_base = best_region.base as usize;
        let bitmap_length = best_region.length as usize;

        debug!(
            RegionAllocator,
            "expected {} bytes, found a region with {} bytes", bytes, bitmap_length
        );

        // allocates and setups bitmap
        let addr = (bitmap_base + crate::limine::get_phy_offset()) as *mut u8;

        let bitmap = unsafe { slice::from_raw_parts_mut(addr, bytes) };

        // setup
        bitmap.fill(0xFF);

        debug_assert!(bitmap[0] == 0xFF);

        let mut this = Self { bitmap };

        debug!(RegionAllocator, "bitmap allocation successful!");
        // sets all unusable frames as used
        for entry in mmap.entries() {
            if entry.entry_type == limine::memory_map::EntryType::USABLE {
                this.set_unused_from(entry.base as PhysAddr, entry.length as usize);
            }

            if entry.base == last_usable_entry.base {
                break;
            }
        }

        this.set_used_from(bitmap_base, bitmap_length);
        this
    }

    #[inline]
    fn set_used_from(&mut self, from: PhysAddr, size: usize) {
        let frames_needed = align_up(size, PAGE_SIZE) / PAGE_SIZE;

        for frame in 0..frames_needed {
            self.set_used(from + frame * PAGE_SIZE);
        }
    }

    #[inline]
    fn set_unused_from(&mut self, from: PhysAddr, size: usize) {
        let frames_needed = align_down(size, PAGE_SIZE) / PAGE_SIZE;

        for frame in 0..frames_needed {
            self.set_unused(from + frame * PAGE_SIZE);
        }
    }

    /// takes a bitmap index(bitnumber) and turns it into (row, col)
    #[inline(always)]
    fn bitmap_loc_from_index(index: usize) -> (usize, usize) {
        (index / 8, index % 8)
    }

    /// takes an addr and turns it into a bitmap (row, col)
    #[inline(always)]
    fn bitmap_loc_from_addr(addr: PhysAddr) -> (usize, usize) {
        Self::bitmap_loc_from_index(align_down(addr, PAGE_SIZE) / PAGE_SIZE)
    }

    pub fn allocate_frame(&mut self) -> Option<Frame> {
        for row in 0..self.bitmap.len() {
            for col in 0..8 {
                if (self.bitmap[row] >> col) & 1 == 0 {
                    self.bitmap[row] |= 1 << col;
                    return Some(Frame {
                        start_address: (row * 8 + col) * PAGE_SIZE,
                    });
                }
            }
        }

        None
    }

    fn set_unused(&mut self, addr: PhysAddr) {
        let (row, col) = Self::bitmap_loc_from_addr(addr);
        self.bitmap[row] ^= 1 << col
    }

    fn set_used(&mut self, addr: PhysAddr) {
        let (row, col) = Self::bitmap_loc_from_addr(addr);
        self.bitmap[row] |= 1 << col
    }

    pub fn deallocate_frame(&mut self, frame: Frame) {
        self.set_unused(frame.start_address);
    }

    pub fn memoy_mapped(&self) -> usize {
        self.bitmap
            .iter()
            .fold(0, |acc, x| acc + x.count_ones() as usize)
    }
}
lazy_static! {
    pub static ref REGION_ALLOCATOR: Mutex<RegionAllocator> = Mutex::new(RegionAllocator::new());
}
#[inline(always)]
pub fn allocate_frame() -> Option<Frame> {
    REGION_ALLOCATOR.lock().allocate_frame()
}

#[inline(always)]
pub fn deallocate_frame(frame: Frame) {
    REGION_ALLOCATOR.lock().deallocate_frame(frame)
}

/// returns the number of mapped frames
#[inline(always)]
pub fn memory_mapped() -> usize {
    REGION_ALLOCATOR.lock().memoy_mapped()
}
