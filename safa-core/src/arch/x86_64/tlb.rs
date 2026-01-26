use core::{
    num::NonZero,
    ptr::NonNull,
    sync::atomic::{self, AtomicUsize, Ordering},
};

use crate::{
    VirtAddr,
    arch::{
        paging::ArchPageTable,
        with_interrupts,
        x86_64::interrupts::{
            InterruptFrame,
            apic::{self, send_eoi},
            handlers::TLBI_ID,
        },
    },
    memory::paging::PAGE_SIZE,
    percpu::{self, CpuLocal},
    scheduler::without_preemption,
    utils::locks::SpinLock,
};

// This is what my system does
/// The number which decides whether or not we should reload CR3 or do invlpg for a given flush range operation.
///
/// It is the number of pages or virtual addresses.
const MAX_INVLPG_FLUSHES: usize = 33;

pub(super) fn handle_tlbi_request() {
    let request_borrow = &DESCRIPTOR.request;

    let request = unsafe { &*request_borrow.data_ptr() };
    request.process();

    assert!(request_borrow.is_locked(), "Got TLBI request without lock");
    unsafe { request_borrow.force_unlock() };
}

pub(super) extern "x86-interrupt" fn tlbi_flush_handler(_: InterruptFrame) {
    handle_tlbi_request();
    send_eoi();
}

struct TLBIDescriptor {
    request: SpinLock<TLBIRequest>,
    responses: AtomicUsize,
}

percpu::define! {
    static DESCRIPTOR: TLBIDescriptor = const {
        TLBIDescriptor {
            request: SpinLock::new(TLBIRequest::new()),
            responses: AtomicUsize::new(0),
        }
    };
}

#[derive(Debug)]
pub(super) struct TLBIRequest {
    page_table: Option<NonNull<ArchPageTable>>,
    range: Option<(VirtAddr, NonZero<usize>)>,
    processed: &'static AtomicUsize,
}

impl TLBIRequest {
    pub const fn new() -> Self {
        static DUMMY: AtomicUsize = AtomicUsize::new(0);
        Self {
            page_table: None,
            range: None,
            processed: &DUMMY,
        }
    }

    pub fn process(&self) {
        if let Some((start, size)) = self.range {
            flush_range(start, start + size.get());
        } else {
            reload_cr3();
        }

        self.processed.fetch_add(1, Ordering::Release);
    }
}

unsafe impl Send for TLBIRequest {}
unsafe impl Sync for TLBIRequest {}

#[inline]
fn flush_range(start: VirtAddr, end: VirtAddr) {
    let mut current = start;
    while current < end {
        invlpg(current);
        current = current + PAGE_SIZE;
    }
}
#[inline]
fn invlpg(addr: VirtAddr) {
    unsafe {
        core::arch::asm!("invlpg ({})", in(reg) addr.into_raw(), options(att_syntax, nostack, preserves_flags))
    };
}

#[inline]
fn reload_cr3() {
    let _tmp: usize;

    unsafe {
        core::arch::asm!(
            "
            mov {0}, cr3
            mov cr3, {0}
            ",
            out(reg) _tmp, options(nostack, preserves_flags),
        )
    }
}

/// Performs a TLB shootdown for addresses starting at `start`, and ending at `end`.
///
/// Exclusive but if start == end, it is going to flush start.
///
/// # Safety
/// Current implementations for all current architectures are safe however, you must make sure `start` and `end` are a multiple of [`PAGE_SIZE`], and end > start.
pub unsafe fn flush_cache_range(page_table: &ArchPageTable, start: VirtAddr, mut end: VirtAddr) {
    let page_table = start
        .is_in_lower_half()
        .then(|| NonNull::from_ref(page_table));

    if start == end {
        end = end + PAGE_SIZE;
    }

    let range = (((end - start) / PAGE_SIZE) <= MAX_INVLPG_FLUSHES)
        .then_some((start, NonZero::new(end - start).expect("end == start")));

    if let Some((start, size)) = range {
        flush_range(start, start + size.get());
    } else {
        reload_cr3();
    }

    let mut expected_waiting = 0;
    let cpus = CpuLocal::get_all();
    if cpus.len_hint() <= 1 {
        return;
    }

    without_preemption(|| {
        // Receive IPIs from other CPUs.
        with_interrupts(|| {
            let current_cpu = CpuLocal::get();

            let curr_descriptor = DESCRIPTOR.borrow_for(current_cpu);
            let responses = &curr_descriptor.responses;
            responses.store(0, Ordering::Relaxed);

            for cpu in cpus.filter(|cpu| cpu.cpu_id != current_cpu.cpu_id) {
                // TODO: Get cpu's page-table?
                // Removed for simplicity and safety

                // let other_pag = cpu.current_pagetable();
                // // If we have got a target we check for it, otherwise we are targeting everyone.
                // if let Some(pag) = page_table
                //     && other_pag.is_none_or(|o| !core::ptr::eq(pag.as_ptr(), o.as_ptr()))
                // {
                //     continue;
                // }

                let next_descriptor = DESCRIPTOR.borrow_for(cpu);

                let mut request = loop {
                    if let Some(request) = next_descriptor.request.try_lock() {
                        break request;
                    }

                    core::hint::spin_loop();
                };

                request.range = range;
                request.page_table = page_table;
                request.processed = responses;

                // Lock will be released when the request is processed.
                core::mem::forget(request);

                atomic::fence(Ordering::Release);
                expected_waiting += 1;

                apic::send_ipi_to(TLBI_ID, cpu.cpu_arch_id);
            }

            while responses.load(Ordering::Acquire) < expected_waiting {
                core::hint::spin_loop()
            }
        })
    });
}
