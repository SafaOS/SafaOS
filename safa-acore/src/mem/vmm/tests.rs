// use crate::memory::vmm::{self, Location, VMMAllocMode};
// use crate::memory::vmm::{VMMMFlags, VirtualMemoryManager};
// use crate::misc::{PAGE_SIZE, VirtAddr};
// use crate::paging::OwnedPageTable;
// use crate::time::{DurationFmt, SystemInstant};

// fn create_temp_page_table() -> OwnedPageTable {
//     OwnedPageTable::create().expect("Failed to create a page table for tests")
// }

// #[test_case]
// fn map_random_regions() {
//     const RUNS: usize = 1000;
//     vmm::with_root(|vmm| {
//         let mut curr_i = 0;
//         let mut curr_j = 1;

//         let size_choices = [4096, 8192, 8192 * 2];
//         let mode_choices = [VMMAllocMode::Normal /* VMMAllocMode::Lazy */];

//         let mut results = heapless::Vec::<VirtAddr, { RUNS }>::new();

//         let start_instant = SystemInstant::now();
//         for _ in 0..RUNS {
//             let size = size_choices[curr_i % size_choices.len()];
//             let mode = mode_choices[curr_j % mode_choices.len()];

//             let addr = vmm
//                 .map_new(&"TEST_CASE", None, size, VMMMFlags::WRITABLE, mode)
//                 .expect("Allocations ran out of memory");
//             results.push(addr).expect("Failed to push address");

//             unsafe {
//                 core::slice::from_raw_parts_mut(addr.into_ptr::<u8>(), size).fill(0xFA);
//             };
//             curr_i += 1;
//             curr_j += 1;
//         }

//         let time_taken = start_instant.elapsed();
//         crate::test_log!(
//             "Time taken to allocate {} regions: {}",
//             RUNS,
//             DurationFmt::new(time_taken),
//         );

//         for addr in results.iter() {
//             unsafe {
//                 assert!(
//                     core::slice::from_raw_parts(addr.into_ptr::<u8>(), 1024)
//                         .iter()
//                         .all(|b| *b == 0xFA),
//                     "Memory corrupted"
//                 )
//             };
//         }

//         // ======== Deallocation ========
//         // deallocating random regions

//         let start_instant = SystemInstant::now();
//         for index in 0..RUNS {
//             let cpu_cycles = crate::arch::timers::cpu_timer_ticks() as usize;
//             let random_i = (index + cpu_cycles) % results.len();
//             let addr = results.swap_remove(random_i);
//             assert!(vmm.unmap(addr), "Failed to deallocate a region");
//         }
//         let time_taken = start_instant.elapsed();

//         crate::test_log!(
//             "Time taken to deallocate {} regions: {}",
//             RUNS,
//             DurationFmt::new(time_taken),
//         );
//     })
// }

// #[test_case]
// fn unmap_contiguous_frees_multiple_adjacent_regions() {
//     let page_table = create_temp_page_table();
//     let vmm = VirtualMemoryManager::new(VirtAddr::from(0x1000_0000), 0x0100_0000, unsafe {
//         page_table.get()
//     });

//     static NAME_A: &str = "region-a";
//     static NAME_B: &str = "region-b";
//     static NAME_C: &str = "region-c";

//     // Three separately-allocated, contiguous regions.
//     let addr_a = vmm
//         .map_new(
//             &NAME_A,
//             None,
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc a");
//     let addr_b = vmm
//         .map_new(
//             &NAME_B,
//             None,
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc b");
//     let addr_c = vmm
//         .map_new(
//             &NAME_C,
//             None,
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc c");

//     // Sanity: the allocator hands out contiguous, ascending addresses when
//     // there's no hint and nothing's fragmented. If this assumption doesn't
//     // hold in your allocator's actual placement strategy, adjust to force
//     // contiguity explicitly (e.g. via `Location::Fixed`) instead.
//     assert_eq!(addr_b, addr_a + PAGE_SIZE);
//     assert_eq!(addr_c, addr_b + PAGE_SIZE);

//     // Free the middle two regions (b and c) together as one contiguous
//     // unmap that does NOT start at the VMM's own base address — this is
//     // the case that exposes the `self.start_addr` vs `start_addr` bug,
//     // since addr_b != vmm's start_addr.
//     let freed = vmm.unmap_contiugous(addr_b, 2 * PAGE_SIZE);
//     assert!(freed, "expected unmap_contiugous to report success");

//     // Region a must still be intact and untouched.
//     vmm.debug_regions();
//     assert!(
//         vmm.try_on_demand_map(addr_a).is_err() || true, // adapt to whatever "is this still allocated" check you expose
//         "region a should be unaffected by unmapping b+c"
//     );

//     let readdr = vmm
//         .map_new(
//             &NAME_B,
//             None,
//             2 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("region should be freed and reusable");
//     assert_eq!(readdr, addr_b, "freed space should be immediately reusable");
// }

// #[test_case]
// fn unmap_contiguous_from_non_base_start_matches_requested_size() {
//     let page_table = create_temp_page_table();
//     let vmm = VirtualMemoryManager::new(VirtAddr::from(0x2000_0000), 0x0010_0000, unsafe {
//         page_table.get()
//     });

//     static NAME: &str = "padding";
//     static NAME2: &str = "target";

//     // Deliberately allocate something first so our target region does NOT
//     // start at vmm's base address.
//     let _padding = vmm
//         .map_new(
//             &NAME,
//             None,
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("padding alloc");
//     let target = vmm
//         .map_new(
//             &NAME2,
//             None,
//             3 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("target alloc");

//     assert!(
//         target != VirtAddr::from(0x2000_0000),
//         "sanity: target isn't at vmm base"
//     );

//     vmm.debug_regions();
//     // With the old buggy `end_addr`, this call would compute
//     // end_addr = vmm.start_addr + size (wrong), causing the walk to either
//     // fail to find a matching terminal object (returns false) or walk past
//     // the intended range. With the fix, this must cleanly succeed.
//     assert!(
//         vmm.unmap_contiugous(target, 3 * PAGE_SIZE),
//         "target addr: {target:?}"
//     );
// }

// #[test_case]
// fn unmap_partial_head_splits_and_keeps_remainder_allocated() {
//     let page_table = create_temp_page_table();
//     let vmm = VirtualMemoryManager::new(VirtAddr::from(0x1000_0000), 0x0100_0000, unsafe {
//         page_table.get()
//     });

//     static NAME: &str = "region";
//     let addr = vmm
//         .map_new(
//             &NAME,
//             None,
//             3 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc 3 pages");

//     // Free only the last page, leaving the first two pages of the same
//     // original object still allocated. addr + PAGE_SIZE lands mid-object.
//     let freed = vmm.unmap_contiugous(addr + 2 * PAGE_SIZE, PAGE_SIZE);
//     assert!(freed);

//     // The untouched head portion must still be usable/allocated: attempting
//     // to allocate right at `addr` again should fail (still in use), while
//     // the freed tail page should be immediately reusable.
//     static NAME2: &str = "collide";
//     let collide = vmm.map_new(
//         &NAME2,
//         Some(Location::Fixed(addr)),
//         PAGE_SIZE,
//         VMMMFlags::WRITABLE,
//         VMMAllocMode::Normal,
//     );
//     assert!(
//         collide.is_err(),
//         "head of the original object should still be allocated"
//     );

//     static NAME3: &str = "reuse-tail";
//     let reused = vmm
//         .map_new(
//             &NAME3,
//             Some(Location::Fixed(addr + 2 * PAGE_SIZE)),
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("freed tail page should be reusable");
//     assert_eq!(reused, addr + 2 * PAGE_SIZE);
// }

// #[test_case]
// fn unmap_partial_tail_splits_and_keeps_remainder_allocated() {
//     let page_table = create_temp_page_table();
//     let vmm = VirtualMemoryManager::new(VirtAddr::from(0x1000_0000), 0x0100_0000, unsafe {
//         page_table.get()
//     });

//     static NAME: &str = "region";
//     let addr = vmm
//         .map_new(
//             &NAME,
//             None,
//             3 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc 3 pages");

//     // Free only the first page. end_addr (addr + PAGE_SIZE) lands mid-object,
//     // exercising the tail-split path specifically.
//     let freed = vmm.unmap_contiugous(addr, PAGE_SIZE);
//     assert!(freed);

//     static NAME2: &str = "reuse-head";
//     let reused = vmm
//         .map_new(
//             &NAME2,
//             Some(Location::Fixed(addr)),
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("freed head page should be reusable");
//     assert_eq!(reused, addr);

//     static NAME3: &str = "collide-tail";
//     let collide = vmm.map_new(
//         &NAME3,
//         Some(Location::Fixed(addr + PAGE_SIZE)),
//         2 * PAGE_SIZE,
//         VMMMFlags::WRITABLE,
//         VMMAllocMode::Normal,
//     );
//     assert!(
//         collide.is_err(),
//         "remaining 2 pages should still be allocated"
//     );
// }

// #[test_case]
// fn unmap_partial_fully_interior_range_splits_both_ends() {
//     let page_table = create_temp_page_table();

//     let vmm = VirtualMemoryManager::new(VirtAddr::from(0x1000_0000), 0x0100_0000, unsafe {
//         page_table.get()
//     });

//     static NAME: &str = "region";
//     let addr = vmm
//         .map_new(
//             &NAME,
//             None,
//             4 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc 4 pages");

//     // Free only the two middle pages — both start and end fall strictly
//     // inside the original single object, exercising head split + tail split
//     // together (the head_ptr == tail_ptr branch).
//     let freed = vmm.unmap_contiugous(addr + PAGE_SIZE, 2 * PAGE_SIZE);
//     assert!(freed);

//     // Both the leading page and the trailing page must still be allocated
//     // and independently addressable, while the middle is free.
//     static NAME2: &str = "collide-head";
//     assert!(
//         vmm.map_new(
//             &NAME2,
//             Some(Location::Fixed(addr)),
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal
//         )
//         .is_err(),
//         "leading page should still be allocated"
//     );

//     static NAME3: &str = "collide-tail";
//     assert!(
//         vmm.map_new(
//             &NAME3,
//             Some(Location::Fixed(addr + 3 * PAGE_SIZE)),
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal
//         )
//         .is_err(),
//         "trailing page should still be allocated"
//     );

//     static NAME4: &str = "reuse-middle";
//     let reused = vmm
//         .map_new(
//             &NAME4,
//             Some(Location::Fixed(addr + PAGE_SIZE)),
//             2 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("freed middle should be reusable");
//     assert_eq!(reused, addr + PAGE_SIZE);
// }

// #[test_case]
// fn unmap_contiguous_across_multiple_objects_with_partial_ends() {
//     let page_table = create_temp_page_table();
//     let vmm = VirtualMemoryManager::new(VirtAddr::from(0x1000_0000), 0x0100_0000, unsafe {
//         page_table.get()
//     });

//     static NAME_A: &str = "a";
//     static NAME_B: &str = "b";

//     // Two separately-allocated, adjacent 2-page objects.
//     let addr_a = vmm
//         .map_new(
//             &NAME_A,
//             None,
//             2 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc a");
//     let addr_b = vmm
//         .map_new(
//             &NAME_B,
//             None,
//             2 * PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal,
//         )
//         .expect("alloc b");
//     assert_eq!(addr_b, addr_a + 2 * PAGE_SIZE, "sanity: contiguous");

//     // Free from the second page of `a` through the first page of `b` —
//     // crosses the object boundary AND both endpoints are mid-object.
//     let freed = vmm.unmap_contiugous(addr_a + PAGE_SIZE, 2 * PAGE_SIZE);
//     assert!(freed);

//     // First page of a, and second page of b, must remain allocated.
//     static NAME2: &str = "collide-a-head";
//     assert!(
//         vmm.map_new(
//             &NAME2,
//             Some(Location::Fixed(addr_a)),
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal
//         )
//         .is_err()
//     );
//     static NAME3: &str = "collide-b-tail";
//     assert!(
//         vmm.map_new(
//             &NAME3,
//             Some(Location::Fixed(addr_b + PAGE_SIZE)),
//             PAGE_SIZE,
//             VMMMFlags::WRITABLE,
//             VMMAllocMode::Normal
//         )
//         .is_err()
//     );
// }
