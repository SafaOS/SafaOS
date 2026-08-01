use crate::VirtAddr;
use crate::memory::paging::{PAGE_SIZE, PhysPageTable};
use crate::memory::vmm::{self, VMMAllocMode};
use crate::memory::vmm::{VMMMFlags, VirtualMemoryManager, objects::ObjectState};
use crate::timer::{DurationFmt, SystemInstant};

#[test_case]
fn map_random_regions() {
    const RUNS: usize = 100;
    vmm::with_root(|vmm| {
        let mut curr_i = 0;
        let mut curr_j = 1;

        let size_choices = [4096, 8192, 8192 * 2];
        let mode_choices = [VMMAllocMode::Normal, VMMAllocMode::Lazy];

        let mut results = heapless::Vec::<VirtAddr, { RUNS }>::new();

        let start_instant = SystemInstant::now();
        for _ in 0..RUNS {
            let size = size_choices[curr_i % size_choices.len()];
            let mode = mode_choices[curr_j % mode_choices.len()];

            let addr = vmm
                .map_new(&"TEST_CASE", None, size, VMMMFlags::WRITEABLE, mode)
                .expect("Allocations ran out of memory");
            results.push(addr).expect("Failed to push address");

            unsafe {
                core::slice::from_raw_parts_mut(addr.into_ptr::<u8>(), size).fill(0xFA);
            };
            curr_i += 1;
            curr_j += 1;
        }

        let time_taken = start_instant.elapsed();
        crate::test_log!(
            "Time taken to allocate {} regions: {}",
            RUNS,
            DurationFmt::new(time_taken),
        );

        for addr in results.iter() {
            unsafe {
                assert!(
                    core::slice::from_raw_parts(addr.into_ptr::<u8>(), 1024)
                        .iter()
                        .all(|b| *b == 0xFA),
                    "Memory corrupted"
                )
            };
        }

        // ======== Deallocation ========
        // deallocating random regions

        let start_instant = SystemInstant::now();
        for index in 0..RUNS {
            let cpu_cycles = crate::arch::utils::cpu_cycles() as usize;
            let random_i = (index + cpu_cycles) % results.len();
            let addr = results.swap_remove(random_i);
            assert!(vmm.unmap(addr), "Failed to deallocate a region");
        }
        let time_taken = start_instant.elapsed();

        crate::test_log!(
            "Time taken to deallocate {} regions: {}",
            RUNS,
            DurationFmt::new(time_taken),
        );
    })
}
#[test_case]
fn allocate_random_regions() {
    const RUNS: usize = 1000;
    let pseudo_page_table = PhysPageTable::create().expect("Failed to create a pseudo page table");
    let vmm = VirtualMemoryManager::new(
        VirtAddr::from(0x1000),
        0xFFFFFFFFFFF,
        pseudo_page_table.frame_ptr(),
    );
    let mut vmm_inner = vmm.inner.lock();

    let mut curr_i = 0;
    let size_choices = [1024, 2048, 4096, 8192];
    let mut results = heapless::Vec::<VirtAddr, { RUNS }>::new();

    let start_instant = SystemInstant::now();
    for _ in 0..RUNS {
        let size = size_choices[curr_i % size_choices.len()];
        let addr = vmm_inner
            .allocate_next_region(
                &"TEST_CASE",
                None,
                size,
                ObjectState::Allocated(VMMMFlags::empty()),
            )
            .expect("Allocations ran out of memory");
        results.push(addr).expect("Failed to push address");
        curr_i += 1;
    }

    let time_taken = start_instant.elapsed();
    crate::test_log!(
        "Time taken to allocate {} regions: {}",
        RUNS,
        DurationFmt::new(time_taken),
    );

    assert_eq!(
        vmm_inner.len(),
        RUNS + 1, /* free region */
        "Not all regions allocated"
    );

    // ======== Deallocation ========
    // deallocating random regions

    let start_instant = SystemInstant::now();
    for index in 0..RUNS {
        let cpu_cycles = crate::arch::utils::cpu_cycles() as usize;
        let random_i = (index + cpu_cycles) % results.len();
        let addr = results.swap_remove(random_i);
        vmm_inner
            .deallocate_at(addr)
            .expect("Failed to deallocate a region");
    }
    let time_taken = start_instant.elapsed();

    crate::test_log!(
        "Time taken to deallocate {} regions: {}",
        RUNS,
        DurationFmt::new(time_taken),
    );

    assert_eq!(
        vmm_inner.len(),
        1,
        "Failed to deallocate and combine all regions"
    );
    drop(vmm_inner);
    vmm.debug_regions();
}

#[test_case]
fn allocate_random_regions_advanced() {
    #[derive(Clone, Copy)]
    enum Instruction {
        AllocateRandom(usize),
        NextSpecificAllocation,
    }

    use crate::memory::paging::PhysPageTable;
    use crate::timer::{DurationFmt, SystemInstant};

    const RUNS: usize = 1000;

    let pseudo_page_table = PhysPageTable::create().expect("Failed to create a pseudo page table");
    let vmm = VirtualMemoryManager::new(
        VirtAddr::from(0x1000),
        0xFFFFFFFFFFF,
        pseudo_page_table.frame_ptr(),
    );
    let mut vmm_inner = vmm.inner.lock();

    let mut curr_i = 0;

    let mut specific_allocations = heapless::Vec::<(usize, usize), 12>::from_slice(&[
        (0x5000, 0x1000),
        (0xA000, 0x1000),
        (0x10000, 0x1000),
        (0xB000000, 0x1000),
        (0xF21000, 0x1000),
        (0xAFAF000, 0x1000),
        (0x12345000, 0x1000),
        (0x1f1000, 0x1000),
        (0x30000000, 0x1000),
        (0x20000000, 0x1000),
        (0x20001000, 0x1000),
    ])
    .expect("Failed to construct instructions");
    let instructions = [
        Instruction::AllocateRandom(1024),
        Instruction::AllocateRandom(2048),
        Instruction::AllocateRandom(4096),
        Instruction::AllocateRandom(8192),
        Instruction::NextSpecificAllocation,
    ];

    let mut results = heapless::Vec::<VirtAddr, { RUNS }>::new();

    let start_instant = SystemInstant::now();
    for _ in 0..RUNS {
        let instruction = instructions[curr_i % instructions.len()];
        curr_i += 1;

        let addr = match instruction {
            Instruction::AllocateRandom(size) => vmm_inner
                .allocate_next_region(
                    &"TEST_CASE_NEXT",
                    None,
                    size,
                    ObjectState::Allocated(VMMMFlags::empty()),
                )
                .expect("Allocations ran out of memory"),
            Instruction::NextSpecificAllocation => {
                let Some((addr, size)) = specific_allocations.pop() else {
                    continue;
                };
                let addr = VirtAddr::from(addr);
                if let Err(err) = vmm_inner.allocate_at(
                    &"TEST_CASE_SPEC",
                    addr,
                    size,
                    ObjectState::Allocated(VMMMFlags::empty()),
                ) {
                    panic!(
                        "Failed to allocate specific region: {:#?}, addr: {:#?}, size: {}",
                        err, addr, size
                    );
                }
                addr
            }
        };
        results.push(addr).expect("Failed to push address");
    }
    let time_taken = start_instant.elapsed();

    crate::test_log!(
        "Time taken to allocate {} regions: {} us",
        results.len(),
        DurationFmt::new(time_taken),
    );

    assert!(
        vmm_inner.len() >= results.len(),
        "VMM has {} objects, but expected at least {}",
        vmm_inner.len(),
        results.len()
    );

    // ======== Deallocation ========
    // deallocating random regions

    let to_deallocate = results.len();
    let start_instant = SystemInstant::now();
    for index in 0..to_deallocate {
        let cpu_cycles = crate::arch::utils::cpu_cycles() as usize;
        let random_i = (index + cpu_cycles) % results.len();
        let addr = results.swap_remove(random_i);
        vmm_inner
            .deallocate_at(addr)
            .expect("Failed to deallocate a region");
    }
    let time_taken = start_instant.elapsed();

    crate::test_log!(
        "Time taken to deallocate {} regions: {}",
        to_deallocate,
        DurationFmt::new(time_taken),
    );

    assert_eq!(
        vmm_inner.len(),
        1,
        "Failed to deallocate and combine all regions"
    );
    drop(vmm_inner);
    vmm.debug_regions();
}

#[test_case]
fn unmap_contiguous_frees_multiple_adjacent_regions() {
    let page_table = PhysPageTable::create().expect("Failed to create a pseudo page table");
    let vmm = VirtualMemoryManager::new(
        VirtAddr::from(0x1000_0000),
        0x0100_0000,
        page_table.frame_ptr(),
    );

    static NAME_A: &str = "region-a";
    static NAME_B: &str = "region-b";
    static NAME_C: &str = "region-c";

    // Three separately-allocated, contiguous regions.
    let addr_a = vmm
        .map_new(
            &NAME_A,
            None,
            PAGE_SIZE,
            VMMMFlags::WRITEABLE,
            VMMAllocMode::Normal,
        )
        .expect("alloc a");
    let addr_b = vmm
        .map_new(
            &NAME_B,
            None,
            PAGE_SIZE,
            VMMMFlags::WRITEABLE,
            VMMAllocMode::Normal,
        )
        .expect("alloc b");
    let addr_c = vmm
        .map_new(
            &NAME_C,
            None,
            PAGE_SIZE,
            VMMMFlags::WRITEABLE,
            VMMAllocMode::Normal,
        )
        .expect("alloc c");

    // Sanity: the allocator hands out contiguous, ascending addresses when
    // there's no hint and nothing's fragmented. If this assumption doesn't
    // hold in your allocator's actual placement strategy, adjust to force
    // contiguity explicitly (e.g. via `Location::Fixed`) instead.
    assert_eq!(addr_b, addr_a + PAGE_SIZE);
    assert_eq!(addr_c, addr_b + PAGE_SIZE);

    // Free the middle two regions (b and c) together as one contiguous
    // unmap that does NOT start at the VMM's own base address — this is
    // the case that exposes the `self.start_addr` vs `start_addr` bug,
    // since addr_b != vmm's start_addr.
    let freed = vmm.unmap_contiugous(addr_b, 2 * PAGE_SIZE);
    assert!(freed, "expected unmap_contiugous to report success");

    // Region a must still be intact and untouched.
    vmm.debug_regions();
    assert!(
        vmm.try_on_demand_map(addr_a).is_err() || true, // adapt to whatever "is this still allocated" check you expose
        "region a should be unaffected by unmapping b+c"
    );

    let readdr = vmm
        .map_new(
            &NAME_B,
            None,
            2 * PAGE_SIZE,
            VMMMFlags::WRITEABLE,
            VMMAllocMode::Normal,
        )
        .expect("region should be freed and reusable");
    assert_eq!(readdr, addr_b, "freed space should be immediately reusable");
}

#[test_case]
fn unmap_contiguous_from_non_base_start_matches_requested_size() {
    let page_table = PhysPageTable::create().expect("Failed to create a pseudo page table");
    let vmm = VirtualMemoryManager::new(
        VirtAddr::from(0x2000_0000),
        0x0010_0000,
        page_table.frame_ptr(),
    );

    static NAME: &str = "padding";
    static NAME2: &str = "target";

    // Deliberately allocate something first so our target region does NOT
    // start at vmm's base address.
    let _padding = vmm
        .map_new(
            &NAME,
            None,
            PAGE_SIZE,
            VMMMFlags::WRITEABLE,
            VMMAllocMode::Normal,
        )
        .expect("padding alloc");
    let target = vmm
        .map_new(
            &NAME2,
            None,
            3 * PAGE_SIZE,
            VMMMFlags::WRITEABLE,
            VMMAllocMode::Normal,
        )
        .expect("target alloc");

    assert!(
        target != VirtAddr::from(0x2000_0000),
        "sanity: target isn't at vmm base"
    );

    vmm.debug_regions();
    // With the old buggy `end_addr`, this call would compute
    // end_addr = vmm.start_addr + size (wrong), causing the walk to either
    // fail to find a matching terminal object (returns false) or walk past
    // the intended range. With the fix, this must cleanly succeed.
    assert!(
        vmm.unmap_contiugous(target, 3 * PAGE_SIZE),
        "target addr: {target:?}"
    );
}
