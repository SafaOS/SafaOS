use core::fmt::Debug;

use crate::misc::{PAGE_SIZE, PhysAddr};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
/// A physical memory Frame (page).
pub struct Frame(PhysAddr);

impl Frame {
    #[inline(always)]
    // Returns the frame that contains a physical address.
    pub const fn containing(address: PhysAddr) -> Self {
        let aligned = address.prev_page();
        Self(aligned)
    }

    #[inline(always)]
    /// Returns the base address of this frame.
    pub const fn addr(&self) -> PhysAddr {
        self.0
    }

    /// Returns the frame next to "after" `self`
    pub const fn next(&self) -> Self {
        Self(self.0 + PAGE_SIZE)
    }

    #[inline(always)]
    /// Returns an iterator over all the physical frames starting at `start` and ending at `end`
    ///
    /// It is an exclusive iterator.
    pub fn iter_frames(start: Frame, end: Frame) -> FrameIter {
        assert!(start.addr() <= end.addr());
        FrameIter { start, end }
    }

    #[inline(always)]
    /// Returns an iterator over all the physical frames starting at `start` and ending at `end`
    ///
    /// It is an exclusive iterator.
    pub fn iter_addresses(start: PhysAddr, end: PhysAddr) -> FrameIter {
        assert!(start <= end);
        Self::iter_frames(Frame::containing(start), Frame::containing(end))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameIter {
    start: Frame,
    end: Frame,
}

impl Iterator for FrameIter {
    type Item = Frame;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start.addr() < self.end.addr() {
            let frame = self.start;

            self.start = self.start.next();
            Some(frame)
        } else {
            None
        }
    }
}

impl ExactSizeIterator for FrameIter {
    fn len(&self) -> usize {
        (self.end.addr() - self.start.addr()) / PAGE_SIZE
    }
}

impl Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Frame")
            .field(&format_args!("{:#x}", *self.addr()))
            .finish()
    }
}
