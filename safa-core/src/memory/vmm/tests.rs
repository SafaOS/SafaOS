use crate::VirtAddr;
use crate::memory::paging::PhysPageTable;
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
