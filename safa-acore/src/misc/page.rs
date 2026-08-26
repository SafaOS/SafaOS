use core::fmt::{Debug, LowerHex};

use crate::misc::{PAGE_SIZE, VirtAddr};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Page {
    base: VirtAddr,
}

impl Debug for Page {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Page({:#x})", *self.base)
    }
}

impl LowerHex for Page {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", *self.base)
    }
}

impl Page {
    /// Returns the page that contains this address.
    pub const fn containing(address: VirtAddr) -> Self {
        Self {
            base: address.prev_page(),
        }
    }

    /// Returns the base address of the page.
    pub const fn addr(&self) -> VirtAddr {
        self.base
    }

    /// Returns the page next to "after" `self`
    pub const fn next(&self) -> Self {
        Self {
            base: self.base + PAGE_SIZE,
        }
    }

    /// Returns an iterator over all the virtual memory pages beginning at `start` and ending at `end`.
    ///
    /// It is an exclusive iter.
    #[inline(always)]
    pub fn iter_pages(start: Page, end: Page) -> IterPage {
        assert!(start.addr() <= end.addr());
        IterPage { start, end }
    }

    #[inline(always)]
    /// Returns an iterator over all the virtual memory pages beginning at `start` and ending at `end`.
    ///
    /// It is an exclusive iter.
    pub fn iter_address(start: VirtAddr, end: VirtAddr) -> IterPage {
        Self::iter_pages(Page::containing(start), Page::containing(end))
    }
}

#[derive(Debug, Clone)]
pub struct IterPage {
    start: Page,
    end: Page,
}

impl IterPage {
    #[inline(always)]
    pub const fn current(&self) -> Page {
        self.start
    }

    #[inline(always)]
    pub const fn end(&self) -> Page {
        self.end
    }

    #[inline(always)]
    pub const fn current_addr(&self) -> VirtAddr {
        self.current().base
    }

    #[inline(always)]
    pub const fn end_addr(&self) -> VirtAddr {
        self.end().base + PAGE_SIZE
    }
}

impl Iterator for IterPage {
    type Item = Page;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start < self.end {
            let page = self.start;

            self.start = self.start.next();
            Some(page)
        } else {
            None
        }
    }
}
