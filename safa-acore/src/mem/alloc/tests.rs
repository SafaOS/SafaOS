//! This file was LLM generated.
//!
//! Tests for `kalloc`/`kfree`, exercising the slab allocator underneath.
//!
//! Assumes a `#[test_case]`-style harness and that `kalloc::init()` has
//! already run. Where possible these lean on `alloc::{Vec, Box, String}`
//! directly — since `KAlloc` is the `#[global_allocator]`, every push,
//! resize, and drop already round-trips through `kalloc`/`kfree`, so we
//! get real-world allocation patterns for free instead of hand-rolling
//! every call site.

extern crate alloc;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::alloc::Layout;

use super::{ALLOC_CACHE_SIZES, KAllocError, kalloc, kfree};

#[test_case]
fn roundtrip() {
    // Every size class, plus an in-between size that must round up.
    for &size in ALLOC_CACHE_SIZES.iter().chain(&[33]) {
        let layout = Layout::from_size_align(size, 1).unwrap();
        let mem = kalloc(layout).unwrap_or_else(|_| panic!("alloc failed for {size}"));

        unsafe { (*mem.as_ptr()).fill((size % 256) as u8) };
        assert!(unsafe { (*mem.as_ptr()).iter().all(|&b| b == (size % 256) as u8) });

        unsafe { kfree(mem.cast::<u8>(), layout) };
    }
}

#[test_case]
fn bad_layout() {
    let too_big = Layout::from_size_align(ALLOC_CACHE_SIZES.last().unwrap() + 1, 1).unwrap();
    assert_eq!(kalloc(too_big).unwrap_err(), KAllocError::LayoutTooBig);

    let bad_align = Layout::from_size_align(8, 16).unwrap();
    assert_eq!(kalloc(bad_align).unwrap_err(), KAllocError::LayoutTooBig);
}

#[test_case]
fn vec_and_string_grow_and_drop() {
    // Growth forces repeated realloc (alloc + copy + free) through kalloc.
    let mut v: Vec<u64> = Vec::new();
    for i in 0..5000 {
        v.push(i);
    }
    assert_eq!(v.iter().sum::<u64>(), (0..5000u64).sum());

    let s = String::from("the quick brown fox ".repeat(200));
    assert_eq!(s.len(), 4000);
    drop(v);
    drop(s);
}

#[test_case]
fn many_boxes_stay_distinct() {
    // Boxed slices across a few size classes, alive at once, checked for
    // aliasing before being dropped (freed) all together.
    let boxes: Vec<Box<[u8]>> = (0u8..=250)
        .map(|i| {
            let mut b = alloc::vec![0u8; 64].into_boxed_slice();
            b.fill(i);
            b
        })
        .collect();

    for (i, b) in boxes.iter().enumerate() {
        assert!(b.iter().all(|&x| x == i as u8), "box {i} was corrupted");
    }
}

#[test_case]
fn magazine_and_slab_list_pressure() {
    // Batch bigger than one magazine (8) to force depot/swap traffic, and
    // enough rounds to cycle slabs through free/partial/full lists.
    let layout = Layout::from_size_align(256, 8).unwrap();

    for _ in 0..50 {
        let batch: Vec<_> = (0..32)
            .map(|_| kalloc(layout).expect("alloc under magazine pressure"))
            .collect();
        for mem in batch {
            unsafe { kfree(mem.cast::<u8>(), layout) };
        }
    }
}

#[test_case]
fn indirect_and_zero_sized() {
    // >= 512 bytes routes through the indirect-slab path.
    let big = Layout::from_size_align(4096, 8).unwrap();
    let mem = kalloc(big).expect("indirect slab alloc");
    assert_eq!(mem.len(), 4096);
    unsafe { kfree(mem.cast::<u8>(), big) };

    // Zero-sized layout should land in the smallest class.
    let zero = Layout::from_size_align(0, 1).unwrap();
    let mem = kalloc(zero).expect("zero-sized alloc");
    assert!(mem.len() >= ALLOC_CACHE_SIZES[0]);
    unsafe { kfree(mem.cast::<u8>(), zero) };
}
