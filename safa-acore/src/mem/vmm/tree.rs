use core::{alloc::Layout, ptr::NonNull};

use alloc::alloc::{AllocError, Allocator};
use libkernel::collections::{LinkedRBTree, QueryFor};

use crate::{
    mem::{
        slab::{INI_SLAB, SlabCacheRef, slab_cache_create},
        vmm::{Location, VMMAllocError, VMMMFlags},
    },
    misc::VirtAddr,
    oninit,
};

#[derive(Debug, Clone, Copy)]
pub struct VMAKey {
    addr: VirtAddr,
    size: usize,
}

impl VMAKey {
    #[inline(always)]
    pub const fn new(addr: VirtAddr, size: usize) -> Self {
        Self { addr, size }
    }

    #[inline(always)]
    pub const fn addr(&self) -> VirtAddr {
        self.addr
    }

    #[inline(always)]
    pub const fn end_addr(&self) -> VirtAddr {
        self.addr() + self.size()
    }

    #[inline(always)]
    pub const fn size(&self) -> usize {
        self.size
    }
}

// TODO: Maybe only partial eq and ord?
impl PartialEq for VMAKey {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Eq for VMAKey {}

impl PartialOrd for VMAKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VMAKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.addr.cmp(&other.addr)
    }
}

impl QueryFor<VMAKey> for VirtAddr {
    fn compare(&self, key: &VMAKey) -> core::cmp::Ordering {
        let addr = key.addr();
        let max_addr = key.end_addr();
        if *self >= max_addr {
            return core::cmp::Ordering::Greater;
        }
        if *self < addr {
            return core::cmp::Ordering::Less;
        }
        core::cmp::Ordering::Equal
    }
}

/// Looks up if any part in a region overlaps with a compared [`VMAKey`].
struct PartialRegionLookup {
    addr: VirtAddr,
    max_addr: VirtAddr,
}

impl QueryFor<VMAKey> for PartialRegionLookup {
    fn compare(&self, key: &VMAKey) -> core::cmp::Ordering {
        // This is more of a lookup then a comparison
        let k_addr = key.addr();
        let k_max_addr = key.end_addr();

        let overlaps = (self.addr < k_max_addr) && (self.max_addr > k_addr);
        if overlaps {
            core::cmp::Ordering::Equal
        } else {
            // If the regions doesn't overlap, continue comparison with the start address
            self.addr.cmp(&key.addr())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMAState {
    Lazy,
    DMA,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VMAEntry {
    name: &'static &'static str,
    state: VMAState,
    flags: VMMMFlags,
}

impl VMAEntry {
    #[inline(always)]
    pub const fn new(name: &'static &'static str, state: VMAState, flags: VMMMFlags) -> Self {
        Self { name, state, flags }
    }

    #[inline(always)]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[inline(always)]
    pub const fn state(&self) -> VMAState {
        self.state
    }

    #[inline(always)]
    pub const fn state_mut(&mut self) -> &mut VMAState {
        &mut self.state
    }

    #[inline(always)]
    pub const fn flags_mut(&mut self) -> &mut VMMMFlags {
        &mut self.flags
    }

    #[inline(always)]
    pub const fn flags(&self) -> VMMMFlags {
        self.flags
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Information about a virtual memory area (VMA).
pub struct VMADescriptor {
    key: VMAKey,
    value: VMAEntry,
}

impl VMADescriptor {
    #[inline(always)]
    pub const fn addr(&self) -> VirtAddr {
        self.key.addr()
    }

    pub const fn size(&self) -> usize {
        self.key.size()
    }

    #[inline(always)]
    pub const fn name(&self) -> &'static str {
        self.value.name()
    }

    #[inline(always)]
    pub const fn state(&self) -> VMAState {
        self.value.state()
    }

    #[inline(always)]
    pub const fn flags(&self) -> VMMMFlags {
        self.value.flags()
    }
}

oninit::define! {
    pub(super) unsafe static VMA_SLAB: SlabCacheRef = with INI_SLAB || {
        slab_cache_create(
                &"VMA",
                Layout::from_size_align(VMATree::SIZE_OF_NODE, 8)
                    .expect("Failed to create layout for VMA node"),
                None,
                None,
            )
            .expect("Failed to create Slab Cache for VMAs")
    };
}

#[derive(Debug, Clone, Copy)]
struct VMAAlloc;

unsafe impl Allocator for VMAAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        _ = layout;
        VMA_SLAB.allocate().map_err(|_| AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        assert_eq!(layout.size(), VMATree::SIZE_OF_NODE);
        VMA_SLAB.free(ptr);
    }
}

pub struct VMATree {
    addr: VirtAddr,
    size: usize,
    tree: LinkedRBTree<VMAKey, VMAEntry, VMAAlloc>,
}

impl core::fmt::Debug for VMATree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VMATree")
            .field("addr", &self.addr)
            .field("size", &self.size)
            .finish()
    }
}

impl VMATree {
    const SIZE_OF_NODE: usize = LinkedRBTree::<VMAKey, VMAEntry, VMAAlloc>::SIZE_OF_NODE;
    const _SIZE_ASSERT: () = assert!(VMATree::SIZE_OF_NODE == 72);

    #[inline(always)]
    pub const fn new(start_addr: VirtAddr, size: usize) -> Self {
        Self {
            addr: start_addr,
            size,
            tree: LinkedRBTree::new_in(VMAAlloc),
        }
    }

    /// Looks up the VMA entry for the given address.
    #[inline]
    pub fn lookup(&self, addr: VirtAddr) -> Option<VMADescriptor> {
        self.tree
            .cursor_to(&addr)
            .key_value()
            .map(|(k, v)| VMADescriptor { key: *k, value: *v })
    }

    #[inline]
    pub fn lookup_mut(&mut self, addr: VirtAddr) -> Option<(VMAKey, &mut VMAEntry)> {
        unsafe {
            self.tree
                .cursor_mut_to(&addr)
                .key_value_mut()
                .map(|(k, v)| (*k, v))
        }
    }

    /// Removes a contiguous range of memory from the tree.
    ///
    /// returns whether the range was removed.
    #[inline(always)]
    pub fn remove_contiguous(&mut self, addr: VirtAddr, size: usize) -> bool {
        self.remove_contiguous_with(addr, size, |_| {})
    }

    /// Removes a contiguous range of memory from the tree.
    ///
    /// returns whether the range was removed.
    pub fn remove_contiguous_with<F: Fn(VMADescriptor)>(
        &mut self,
        addr: VirtAddr,
        size: usize,
        f: F,
    ) -> bool {
        // We want to do the following:
        // - Remove all regions that entirely are within the range [addr, max_addr)
        // - Partially remove the first region that is partially within the range [addr, max_addr)
        // - Partially remove the last region that is partially within the range [addr, max_addr) (Done)

        let mut removed = false;
        self.modify_contiguous_or_remove(addr, size, |addr, size, value| {
            removed = true;
            f(VMADescriptor {
                key: VMAKey::new(addr, size),
                value: *value,
            });
            None
        })
        .expect("Shouldn't allocate");
        removed
    }

    #[inline]
    /// Modifies a contiguous range of memory in the tree with the result of the callback, or removes it if the callback returns `None`.
    ///
    /// May allocate if we need to split a region to satisfy a request of modification.
    pub fn modify_contiguous_or_remove<F: FnMut(VirtAddr, usize, &VMAEntry) -> Option<VMAEntry>>(
        &mut self,
        r_addr: VirtAddr,
        size: usize,
        mut f: F,
    ) -> Result<(), AllocError> {
        let r_max_addr = r_addr + size;
        let mut cursor = self.tree.cursor_mut_to(&r_addr);
        if cursor.key().is_none() {
            return Ok(());
        }

        let mut back_overlapping = None;
        let mut front_overlapping = None;

        loop {
            let key_value = unsafe { cursor.key_value_mut() };
            let Some((k, v)) = key_value else {
                break;
            };

            if k.addr() >= r_addr && k.end_addr() <= r_max_addr {
                // entirely within the range [addr, max_addr)
                if let Some(new_entry) = f(k.addr(), k.size(), v) {
                    *v = new_entry;
                } else {
                    cursor.remove_inplace();
                }
            } else if r_addr > k.addr() && r_max_addr < k.end_addr() {
                assert_eq!(front_overlapping, None);
                assert_eq!(back_overlapping, None);
                // The range [addr, max_addr) is entirely within region
                // We want to split from both ends
                // the one in the middle would be the new entry
                // front_overlapping and back_overlapping are the two parts to be split
                //
                // However for sake of performance in case of removal we will only use back_overlapping and the region will be set to front_overlapping
                let new_entry = f(r_addr, r_max_addr - r_addr, v);

                let old_k_max_addr = k.end_addr();
                let old_k_addr = k.addr();

                if let Some(new_entry) = new_entry {
                    k.addr = r_addr;
                    // set to r.size
                    k.size = r_max_addr - r_addr;
                    front_overlapping = Some(VMADescriptor {
                        key: VMAKey::new(old_k_addr, r_addr - old_k_addr),
                        value: *v,
                    });

                    back_overlapping = Some(VMADescriptor {
                        key: VMAKey::new(r_max_addr, old_k_max_addr - r_max_addr),
                        value: *v,
                    });

                    *v = new_entry;
                } else {
                    k.size = r_addr - k.addr();
                    back_overlapping = Some(VMADescriptor {
                        key: VMAKey::new(r_max_addr, old_k_max_addr - r_max_addr),
                        value: *v,
                    });
                }
                break;
            } else if k.addr() >= r_addr && k.addr() < r_max_addr {
                // this means k.end_addr() > max_addr
                // We can introduce a gap by setting k.addr() to max_addr
                // it still covers K's range.
                //
                // We will spilt this region
                let old_addr = k.addr();
                let old_max_addr = k.end_addr();

                // Remove the overflowing part
                k.addr = r_max_addr;
                k.size = old_max_addr - r_max_addr;

                if let Some(new_entry) = f(old_addr, r_max_addr - old_addr, v) {
                    assert_eq!(
                        back_overlapping, None,
                        "Shouldn't have 2 back overlapping nodes"
                    );
                    back_overlapping = Some(VMADescriptor {
                        key: VMAKey::new(old_addr, r_max_addr - old_addr),
                        value: new_entry,
                    });
                }
            } else if k.end_addr() > r_addr && k.addr() < r_addr {
                let old_max_addr = k.end_addr();
                // k overflows into the region
                k.size = r_addr - k.addr();

                if let Some(new_entry) = f(r_addr, old_max_addr - r_addr, v) {
                    assert_eq!(
                        front_overlapping, None,
                        "Shouldn't have 2 front overlapping nodes"
                    );
                    front_overlapping = Some(VMADescriptor {
                        key: VMAKey::new(r_addr, old_max_addr - r_addr),
                        value: new_entry,
                    });
                }
            } else {
                break;
            }

            cursor.move_next();
        }

        if let Some(front_overlapping) = front_overlapping {
            self.insert(front_overlapping)?;
        }

        if let Some(back_overlapping) = back_overlapping {
            self.insert(back_overlapping)?;
        }

        Ok(())
    }

    #[inline]
    /// Removes the entry containing `addr` from the tree, returning it if found.
    pub fn remove_containing(&mut self, addr: VirtAddr) -> Option<VMADescriptor> {
        self.tree
            .remove(&addr)
            .map(|(k, v)| VMADescriptor { key: k, value: v })
    }

    #[inline]
    fn insert(&mut self, desc: VMADescriptor) -> Result<(), AllocError> {
        self.tree.try_insert(desc.key, desc.value).map(|_| ())
    }

    /// Looks up a gap at least `size` bytes starting at `location` if given.
    fn lookup_gap(
        &self,
        location: Option<Location>,
        size: usize,
    ) -> Result<(VirtAddr, usize), VMMAllocError> {
        let max_addr = self.addr + self.size;
        let mut cursor;
        let mut prev_max_addr = self.addr;
        match location {
            None => cursor = self.tree.front_cursor(),
            Some(Location::Fixed(f)) => {
                // We want to lookup from f to f + size, if no address is found we got a gap.
                return match self.tree.get(&PartialRegionLookup {
                    addr: f,
                    max_addr: f + size,
                }) {
                    Some(_) => Err(VMMAllocError::Used),
                    None => Ok((f, size)),
                };
            }
            Some(Location::Hint(h)) => {
                cursor = self.tree.cursor_to(&h);
                if cursor.key().is_none() {
                    // Move to head
                    cursor.move_next();
                } else {
                    prev_max_addr = cursor
                        .peek_prev()
                        .map(|(k, _)| k.end_addr())
                        .unwrap_or(self.addr);
                }
            }
        };

        loop {
            let gap_size;
            let curr_max_addr;

            if let Some(key) = cursor.key() {
                gap_size = key.addr() - prev_max_addr;
                curr_max_addr = Some(key.end_addr());
            } else {
                gap_size = max_addr - prev_max_addr;
                curr_max_addr = None;
            }

            if gap_size >= size {
                return Ok((prev_max_addr, gap_size));
            }

            if let Some(curr_max_addr) = curr_max_addr {
                prev_max_addr = curr_max_addr;
                cursor.move_next();
            } else {
                break Err(VMMAllocError::OutOfMemory);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = VMADescriptor> {
        let mut cursor = self.tree.front_cursor();

        core::iter::from_fn(move || {
            let kv = cursor.key_value();
            cursor.move_next();
            kv.map(|(key, value)| VMADescriptor {
                key: *key,
                value: *value,
            })
        })
    }

    /// Allocates a gap in the tree, returning the address of the gap.
    ///
    /// The gap is allocated according to [`Location`].
    pub fn allocate_gap(
        &mut self,
        location: Option<Location>,
        size: usize,
        entry: VMAEntry,
    ) -> Result<VirtAddr, VMMAllocError> {
        let (addr, _) = self.lookup_gap(location, size)?;

        self.insert(VMADescriptor {
            key: VMAKey { addr, size },
            value: entry,
        })?;
        Ok(addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_VMA_ENTRY: VMAEntry = VMAEntry::new(&"TEST", VMAState::Normal, VMMMFlags::empty());
    const LAZY_ENTRY: VMAEntry = VMAEntry::new(&"LAZY", VMAState::Lazy, VMMMFlags::empty());

    fn ascending_with_states(tree: &VMATree) -> alloc::vec::Vec<(VirtAddr, usize, VMAState)> {
        tree.iter()
            .map(|desc| (desc.addr(), desc.size(), desc.state()))
            .collect::<alloc::vec::Vec<_>>()
    }

    fn ascending(tree: &VMATree) -> alloc::vec::Vec<(VirtAddr, usize)> {
        tree.iter()
            .map(|desc| (desc.addr(), desc.size()))
            .collect::<alloc::vec::Vec<_>>()
    }

    fn tree_with(regions: &[(usize, usize)]) -> VMATree {
        let mut tree = VMATree::new(VirtAddr::new(0), 0x1000);
        for &(addr, size) in regions {
            tree.allocate_gap(
                Some(Location::Fixed(VirtAddr::new(addr))),
                size,
                TEST_VMA_ENTRY,
            )
            .expect("Failed to allocate a gap");
        }
        tree
    }

    #[test_case]
    fn a_allocate_gap() {
        let mut tree = VMATree::new(VirtAddr::new(0), 0x1000);
        let results = tree.allocate_gap(None, 0x100, TEST_VMA_ENTRY);

        assert_eq!(results, Ok(VirtAddr::new(0)));

        let results = tree.allocate_gap(None, 0x200, TEST_VMA_ENTRY);

        assert_eq!(results, Ok(VirtAddr::new(0x100)));

        let results = tree.allocate_gap(None, 0x300, TEST_VMA_ENTRY);

        assert_eq!(results, Ok(VirtAddr::new(0x300)));

        let results = tree.allocate_gap(None, 0xa01, TEST_VMA_ENTRY);

        assert_eq!(
            results,
            Err(VMMAllocError::OutOfMemory),
            "there should be no gap with that size"
        );
    }

    #[test_case]
    fn b_allocate_gap_with_hint() {
        let mut tree = VMATree::new(VirtAddr::new(0), 0x1000);
        // Allocate at a fixed address
        let results = tree.allocate_gap(
            Some(Location::Fixed(VirtAddr::new(0x100))),
            0x100,
            TEST_VMA_ENTRY,
        );

        assert_eq!(results, Ok(VirtAddr::new(0x100)));

        // Allocate without a hint after the fixed address
        let results = tree.allocate_gap(None, 0x300, TEST_VMA_ENTRY);
        assert_eq!(results, Ok(VirtAddr::new(0x200)));

        // Allocate with the hint at the location we just allocated.
        let results = tree.allocate_gap(
            Some(Location::Hint(VirtAddr::new(0x200))),
            0x100,
            TEST_VMA_ENTRY,
        );
        assert_eq!(results, Ok(VirtAddr::new(0x500)));

        // Allocate without a hint before all the allocated regions
        let results = tree.allocate_gap(None, 0x100, TEST_VMA_ENTRY);
        assert_eq!(results, Ok(VirtAddr::null()));

        assert_eq!(
            ascending(&tree),
            &[
                (VirtAddr::null(), 0x100),
                (VirtAddr::new(0x100), 0x100),
                (VirtAddr::new(0x200), 0x300),
                (VirtAddr::new(0x500), 0x100),
            ]
        );
    }

    #[test_case]
    fn c_deallocate_contiguous() {
        let mut tree = VMATree::new(VirtAddr::new(0), 0x1000);

        // Simple case, region entirely within range.
        assert_eq!(
            tree.allocate_gap(None, 0x100, TEST_VMA_ENTRY),
            Ok(VirtAddr::null())
        );
        tree.remove_contiguous(VirtAddr::null(), 0x100);
        assert_eq!(ascending(&tree), &[]);

        tree.allocate_gap(None, 0x1000, TEST_VMA_ENTRY)
            .expect("Failed to allocate a gap");

        // Range entirely within region.
        tree.remove_contiguous(VirtAddr::new(0x200), 0x300);
        assert_eq!(
            ascending(&tree),
            &[(VirtAddr::null(), 0x200), (VirtAddr::new(0x500), 0xb00),]
        );

        // Nothing to remove.
        assert_eq!(
            tree.remove_contiguous(VirtAddr::new(0x200), 0x300),
            false,
            "No regions should be in that range"
        );
        assert_eq!(tree.tree.len(), 2);

        // Region overflows out of range.
        tree.remove_contiguous(VirtAddr::null(), 0x100);
        // Region overflows into range.
        tree.remove_contiguous(VirtAddr::new(0xe00), 0x200);

        assert_eq!(
            ascending(&tree),
            &[(VirtAddr::new(0x100), 0x100), (VirtAddr::new(0x500), 0x900)]
        );
    }

    #[test_case]
    fn d_allocate_fixed() {
        let mut tree = VMATree::new(VirtAddr::new(0), 0x1000);
        assert_eq!(
            tree.allocate_gap(
                Some(Location::Hint(VirtAddr::new(0x300))),
                0x100,
                TEST_VMA_ENTRY,
            ),
            Ok(VirtAddr::new(0x0))
        );

        assert_eq!(
            tree.allocate_gap(
                Some(Location::Fixed(VirtAddr::new(0x300))),
                0x100,
                TEST_VMA_ENTRY
            ),
            Ok(VirtAddr::new(0x300)),
        );

        assert_eq!(
            tree.allocate_gap(
                Some(Location::Fixed(VirtAddr::new(0x300))),
                0x100,
                TEST_VMA_ENTRY
            ),
            Err(VMMAllocError::Used),
            "Allocation should fail at existing fixed location"
        );

        assert_eq!(
            tree.allocate_gap(
                Some(Location::Fixed(VirtAddr::new(0x200))),
                0x200,
                TEST_VMA_ENTRY
            ),
            Err(VMMAllocError::Used),
            "Allocation should fail at existing fixed location"
        );

        assert_eq!(
            tree.allocate_gap(
                Some(Location::Fixed(VirtAddr::new(0x200))),
                0x100,
                TEST_VMA_ENTRY
            ),
            Ok(VirtAddr::new(0x200)),
            "alloc failed tree: {:?}",
            ascending(&tree),
        );
    }

    // This one is AI generated.
    #[test_case]
    fn e_modify_tree() {
        use VMAState::{Lazy, Normal};
        let a = VirtAddr::new;
        let modify_to_lazy = |_, _, _: &VMAEntry| Some(LAZY_ENTRY);

        // Region entirely within range: value is replaced, geometry untouched.
        let mut tree = tree_with(&[(0, 0x1000)]);
        tree.modify_contiguous_or_remove(a(0), 0x1000, modify_to_lazy)
            .expect("Shouldn't allocate");
        assert_eq!(ascending_with_states(&tree), &[(a(0), 0x1000, Lazy)]);

        // Range entirely within region: split into three (front, modified middle, back).
        let mut tree = tree_with(&[(0, 0x1000)]);
        tree.modify_contiguous_or_remove(a(0x200), 0x300, modify_to_lazy)
            .expect("Failed to split region");
        assert_eq!(
            ascending_with_states(&tree),
            &[
                (a(0), 0x200, Normal),
                (a(0x200), 0x300, Lazy),
                (a(0x500), 0xb00, Normal),
            ]
        );

        // Region overflows out of the range (range covers the start of the region).
        let mut tree = tree_with(&[(0, 0x1000)]);
        tree.modify_contiguous_or_remove(a(0), 0x400, modify_to_lazy)
            .expect("Failed to split region");
        assert_eq!(
            ascending_with_states(&tree),
            &[(a(0), 0x400, Lazy), (a(0x400), 0xc00, Normal)]
        );

        // Region overflows into the range (range covers the end of the region).
        let mut tree = tree_with(&[(0, 0x1000)]);
        tree.modify_contiguous_or_remove(a(0xc00), 0x400, modify_to_lazy)
            .expect("Failed to split region");
        assert_eq!(
            ascending_with_states(&tree),
            &[(a(0), 0xc00, Normal), (a(0xc00), 0x400, Lazy)]
        );

        // Range spans several regions: partial front, full middle, partial back.
        let mut tree = tree_with(&[(0, 0x400), (0x400, 0x400), (0x800, 0x400)]);
        tree.modify_contiguous_or_remove(a(0x200), 0x800, modify_to_lazy)
            .expect("Failed to split regions");
        assert_eq!(
            ascending_with_states(&tree),
            &[
                (a(0), 0x200, Normal),
                (a(0x200), 0x200, Lazy),
                (a(0x400), 0x400, Lazy),
                (a(0x800), 0x200, Lazy),
                (a(0xa00), 0x200, Normal),
            ]
        );

        // Removal: range entirely within region leaves a hole.
        let mut tree = tree_with(&[(0, 0x1000)]);
        tree.modify_contiguous_or_remove(a(0x200), 0x300, |_, _, _| None)
            .expect("Failed to split region");
        assert_eq!(
            ascending_with_states(&tree),
            &[(a(0), 0x200, Normal), (a(0x500), 0xb00, Normal)]
        );

        // Removal spanning several regions, trimming both ends.
        let mut tree = tree_with(&[(0, 0x400), (0x400, 0x400), (0x800, 0x400)]);
        tree.modify_contiguous_or_remove(a(0x200), 0x800, |_, _, _| None)
            .expect("Shouldn't allocate");
        assert_eq!(
            ascending_with_states(&tree),
            &[(a(0), 0x200, Normal), (a(0xa00), 0x200, Normal)]
        );

        // Removal of several regions entirely within range. This checks that
        // `remove_inplace` followed by `move_next` doesn't skip a node.
        let mut tree = tree_with(&[(0, 0x400), (0x400, 0x400), (0x800, 0x400)]);
        tree.modify_contiguous_or_remove(a(0), 0x1000, |_, _, _| None)
            .expect("Shouldn't allocate");
        assert_eq!(ascending_with_states(&tree), &[]);

        // Callback sees each affected region exactly once; untouched regions are not visited.
        let mut tree = tree_with(&[(0, 0x100), (0x100, 0x100), (0x800, 0x100)]);
        let mut calls = 0;
        tree.modify_contiguous_or_remove(a(0), 0x200, |_, _, e| {
            calls += 1;
            Some(*e)
        })
        .expect("Shouldn't allocate");
        assert_eq!(calls, 2);

        // Nothing in range: callback is never called and the tree is unchanged.
        let mut tree = tree_with(&[(0, 0x100)]);
        let mut calls = 0;
        tree.modify_contiguous_or_remove(a(0x500), 0x100, |_, _, e| {
            calls += 1;
            Some(*e)
        })
        .expect("Shouldn't allocate");
        assert_eq!(calls, 0);
        assert_eq!(ascending_with_states(&tree), &[(a(0), 0x100, Normal)]);
    }

    #[test_case]
    fn f_allocate_respects_bounds() {
        let mut tree = VMATree::new(VirtAddr::new(0x230), 0x1000);
        let results = tree.allocate_gap(None, 0x1000, TEST_VMA_ENTRY);
        assert_eq!(results, Ok(VirtAddr::new(0x230)));
        tree.remove_contiguous(VirtAddr::new(0x230), 0x100);

        let results = tree.allocate_gap(None, 0x100, TEST_VMA_ENTRY);
        assert_eq!(results, Ok(VirtAddr::new(0x230)));

        tree.remove_contiguous(VirtAddr::new(0x230), 0x1000);

        let results = tree.allocate_gap(None, 0x100, TEST_VMA_ENTRY);
        assert_eq!(results, Ok(VirtAddr::new(0x230)));
    }
}
