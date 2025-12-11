use core::{
    num::NonZero,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    VirtAddr,
    arch::{
        paging::PageTable,
        smp::CPULocal,
        x86_64::{
            interrupts::{
                InterruptFrame,
                apic::{self, send_eoi},
                handlers::TLBI_ID,
            },
            threading::without_yielding,
        },
    },
    memory::paging::PAGE_SIZE,
    utils::locks::SpinLock,
};

// This is what my system does
/// The number which decides whether or not we should reload CR3 or do invlpg for a given flush range operation.
///
/// It is the number of pages or virtual addresses.
const MAX_INVLPG_FLUSHES: usize = 33;

pub(super) fn handle_tlbi_request() {
    let request = unsafe { super::smp::CPULocal::get_current().tlbi_request_read() };

    request.process();
    send_eoi();
}

pub(super) extern "x86-interrupt" fn tlbi_flush_handler(_: InterruptFrame) {
    handle_tlbi_request()
}

#[derive(Debug)]
pub(super) struct TLBIRequest {
    pub shootdown_lock: SpinLock<()>,
    page_table: Option<NonNull<PageTable>>,
    range: Option<(VirtAddr, NonZero<usize>)>,
    processed: &'static AtomicUsize,
}

impl TLBIRequest {
    pub fn new() -> Self {
        static DUMMY_PROCESSED: AtomicUsize = AtomicUsize::new(0);

        Self {
            shootdown_lock: SpinLock::new(()),
            page_table: None,
            range: None,
            processed: &DUMMY_PROCESSED,
        }
    }

    pub fn process(&self) {
        if let Some((start, size)) = self.range {
            flush_range(start, start + size.get());
        } else {
            reload_cr3();
        }

        debug_assert!(
            self.shootdown_lock.is_locked(),
            "Shootdown lock wasn't picked"
        );
        self.processed.fetch_add(1, Ordering::Relaxed);
        unsafe {
            self.shootdown_lock.force_unlock();
        }
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
pub unsafe fn flush_cache_range(page_table: &PageTable, start: VirtAddr, mut end: VirtAddr) {
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

    without_yielding(|current_cpu| {
        let cpus = CPULocal::get_all();

        if cpus.len() > 1 {
            current_cpu.responses_count.store(0, Ordering::Relaxed);

            for cpu in cpus {
                if core::ptr::eq(*cpu, current_cpu) {
                    continue;
                }

                let other_pag = cpu.current_pagetable();
                // If we have got a target we check for it, otherwise we are targeting everyone.
                if let Some(pag) = page_table
                    && other_pag.is_none_or(|o| !core::ptr::eq(pag.as_ptr(), o.as_ptr()))
                {
                    continue;
                }

                unsafe {
                    let tlb = cpu.tlbi_request_lock();
                    tlb.range = range;
                    tlb.page_table = page_table;
                    tlb.processed = &current_cpu.responses_count;
                }

                expected_waiting += 1;
                apic::send_ipi_to(TLBI_ID, cpu.cpuid());
            }

            while current_cpu.responses_count.load(Ordering::Relaxed) < expected_waiting {
                core::hint::spin_loop()
            }
        }
    });
}
