use core::{
    cell::{SyncUnsafeCell, UnsafeCell},
    mem::{MaybeUninit, offset_of},
    ptr::NonNull,
    sync::atomic::AtomicUsize,
};

use alloc::sync::Arc;
use core::arch::asm;
use limine::mp::Cpu;

use crate::{
    VirtAddr,
    arch::{
        self,
        paging::{CURRENT_RING0_PAGE_TABLE, PageTable, set_current_page_table_phys},
        registers::CPUID,
        threading::{CPUStatus, restore_cpu_status},
        x86_64::{gdt::TaskStateSegment, registers::wrmsr, tlb::TLBIRequest},
    },
    debug,
    limine::MP_RESPONSE,
    memory::frame_allocator::FramePtr,
    process::Process,
    scheduler::Scheduler,
    thread::ContextPriority,
};

static BOOT_CORE_ARGS: SyncUnsafeCell<MaybeUninit<(Arc<Process>, fn() -> !)>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());

pub static READY_CPUS: AtomicUsize = AtomicUsize::new(1);

#[repr(C)]
pub struct CPULocal {
    scheduler: Option<Scheduler>,

    tss_ptr: NonNull<TaskStateSegment>,
    tlb_request: UnsafeCell<TLBIRequest>,
    cpuid: CPUID,
    /// !!!!! TODO: REMOVE WHEN THE STACK IS IN THE HIGHER-HALF !!!!!!
    ///
    /// This is used when an operation on this CPU needs to wait for other CPUS' responses.
    pub responses_count: AtomicUsize,
    ptr_to_self: *const Self,
    can_thread_yield: UnsafeCell<bool>,
}

impl CPULocal {
    #[inline]
    pub(super) unsafe fn disable_yielding(&self) -> bool {
        unsafe { self.can_thread_yield.get().replace(false) }
    }

    #[inline]
    pub(super) unsafe fn set_yield_enable(&self, enable_yielidng: bool) {
        unsafe { *self.can_thread_yield.get() = enable_yielidng }
    }

    #[inline]
    pub fn can_thread_yield(&self) -> bool {
        unsafe { *self.can_thread_yield.get() }
    }

    #[inline]
    pub(super) unsafe fn tlbi_request_lock<'a>(&'a self) -> &'a mut TLBIRequest {
        let r = unsafe { self.tlb_request.as_ref_unchecked() };
        // Will unlock self once processed
        core::mem::forget(r.shootdown_lock.lock());
        unsafe { self.tlb_request.as_mut_unchecked() }
    }

    #[inline]
    pub(super) unsafe fn tlbi_request_read<'a>(&'a self) -> &'a TLBIRequest {
        let r = unsafe { self.tlb_request.as_ref_unchecked() };
        r
    }

    #[inline]
    pub(super) const fn cpuid(&self) -> CPUID {
        self.cpuid
    }

    /// If the scheduler is initialized returns the current thread's PID.
    ///
    /// Safety: Current pid may change during or after this call.
    #[inline]
    pub(super) fn current_pagetable(&self) -> Option<FramePtr<PageTable>> {
        self.scheduler()
            .map(|s| unsafe { s.current_thread_ref().process().page_table() })
    }

    fn new(tss_ptr: NonNull<TaskStateSegment>) -> Self {
        Self {
            tss_ptr,
            tlb_request: UnsafeCell::new(TLBIRequest::new()),
            scheduler: None,
            cpuid: unsafe { CPUID::new(0) },
            responses_count: AtomicUsize::new(0),
            can_thread_yield: UnsafeCell::new(true),
            ptr_to_self: core::ptr::null(),
        }
    }

    pub fn on_allocated(&mut self) {
        self.ptr_to_self = self;
    }

    fn set_scheduler(&'static mut self, schd: Scheduler) {
        assert!(self.scheduler.is_none(), "Scheduler already initialized");
        let schd_mut = self.scheduler.insert(schd);
        unsafe { schd_mut.idle_thread().set_scheduler(schd_mut) }
    }

    /// Returns the scheduler
    #[inline(always)]
    pub fn scheduler(&self) -> Option<&Scheduler> {
        self.scheduler.as_ref()
    }

    /// Gets a ptr to the current CPU local
    #[inline]
    pub unsafe fn get_current_ptr() -> NonNull<Self> {
        let ptr: *mut Self;
        unsafe { core::arch::asm!("mov {}, gs:0", out(reg) ptr) }
        unsafe { NonNull::new_unchecked(ptr) }
    }

    /// Gets a reference to the current CPU local
    ///
    /// Safety: should succeed as long as we are past the first ever arch init, so it is marked safe
    #[inline]
    pub fn get_current() -> &'static Self {
        unsafe { Self::get_current_ptr().as_ref() }
    }

    /// Returns a muttable reference to the TSS within this CPU
    ///
    #[inline]
    pub(super) unsafe fn tss_mut(&self) -> &mut TaskStateSegment {
        unsafe { &mut *self.tss_ptr.as_ptr() }
    }
}

unsafe impl Send for CPULocal {}
unsafe impl Sync for CPULocal {}

unsafe fn set_gs(value: VirtAddr) {
    unsafe {
        wrmsr(0xC0000101, value.into_raw() as u64);
        wrmsr(0xC0000102, value.into_raw() as u64);
        asm!("swapgs");
    }
}

fn add_cpu_local(local: CPULocal) -> NonNull<CPULocal> {
    let ptr = crate::arch::smp_misc::allocate_cpu_local(local);
    NonNull::from_mut(ptr)
}

/// Initializes CPU Local storage
pub fn init_cpu_local(tss_ptr: NonNull<TaskStateSegment>) {
    let new = CPULocal::new(tss_ptr);
    let ptr = add_cpu_local(new);

    unsafe {
        set_gs(VirtAddr::from_ptr(ptr.as_ptr()) + offset_of!(CPULocal, ptr_to_self));
    }
}

pub unsafe fn set_cpu_id() {
    unsafe {
        CPULocal::get_current_ptr().as_mut().cpuid = CPUID::get();
    }
}

/// Sets the current CPU's scheduler
fn set_local_scheduler(scheduler: Scheduler) {
    unsafe {
        CPULocal::get_current_ptr()
            .as_mut()
            .set_scheduler(scheduler);
    }
}

/// Initializes the local scheduler with an IDLE thread
fn init_local_scheduler(process: &Arc<Process>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let (idle_thread, _) = process
        .threads_manager()
        .create_thread(
            process,
            VirtAddr::from(idle_function as usize),
            VirtAddr::null(),
            Some(ContextPriority::Low),
            None,
        )
        .expect("Failed to create the idle thread for a CPU");

    let status = unsafe { idle_thread.context_unchecked().cpu_status() };
    let scheduler = Scheduler::new(idle_thread);
    set_local_scheduler(scheduler);
    status
}

fn boot_cpu_inner(lapic_id: u8, process: &Arc<Process>, idle_function: fn() -> !) -> ! {
    unsafe {
        debug!("setting up CPU with lapic ID: {lapic_id}");
        let status_ref = init_local_scheduler(process, idle_function).as_ref();

        debug!(
            "CPU with lapic ID {}: jumping to {:#x}, with stack at {:#x}",
            lapic_id,
            status_ref.at(),
            status_ref.stack_at()
        );
        READY_CPUS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        restore_cpu_status(status_ref)
    }
}

extern "C" fn boot_cpu(cpu: &Cpu) -> ! {
    arch::without_interrupts(|| {
        let tss = arch::x86_64::setup_cpu_generic0();

        unsafe {
            let phys_addr = *CURRENT_RING0_PAGE_TABLE.get();
            set_current_page_table_phys(phys_addr);

            arch::x86_64::setup_cpu_generic1(tss);
            arch::x86_64::setup_cpu_generic2();

            let (process, idle_function) = (*BOOT_CORE_ARGS.get()).assume_init_ref();
            boot_cpu_inner(cpu.lapic_id as u8, process, *idle_function)
        }
    })
}

pub unsafe fn init_cpus(process: &Arc<Process>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let cpus = (*MP_RESPONSE).cpus();
    crate::arch::smp_misc::reserve_cpus(cpus.len() - 1);

    // the current CPU should take local 0
    unsafe { *BOOT_CORE_ARGS.get() = MaybeUninit::new((process.clone(), idle_function)) };

    for cpu in &cpus[1..] {
        cpu.goto_address.write(boot_cpu);
    }

    super::with_interrupts(|| {
        while READY_CPUS.load(core::sync::atomic::Ordering::Relaxed) != cpus.len() {
            core::hint::spin_loop();
        }
    });

    init_local_scheduler(process, idle_function)
}
