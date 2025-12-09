use core::{
    cell::SyncUnsafeCell,
    mem::{MaybeUninit, offset_of},
    num::NonZero,
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
        paging::{CURRENT_RING0_PAGE_TABLE, set_current_page_table_phys},
        threading::{CPUStatus, restore_cpu_status},
        x86_64::{gdt::TaskStateSegment, registers::wrmsr},
    },
    debug,
    limine::MP_RESPONSE,
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
    tsc_frequency: NonZero<u64>,
    ptr_to_self: *const Self,
}

impl CPULocal {
    fn new(tss_ptr: NonNull<TaskStateSegment>, tsc_frequency: NonZero<u64>) -> Self {
        Self {
            tsc_frequency,
            tss_ptr,
            scheduler: None,
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
    /// Returns the frequency of the TSC
    #[inline]
    pub(super) fn tsc_freq(&self) -> NonZero<u64> {
        self.tsc_frequency
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
pub fn init_cpu_local(tss_ptr: NonNull<TaskStateSegment>, tsc_frequency: NonZero<u64>) {
    let new = CPULocal::new(tss_ptr, tsc_frequency);
    let ptr = add_cpu_local(new);

    unsafe {
        set_gs(VirtAddr::from_ptr(ptr.as_ptr()) + offset_of!(CPULocal, ptr_to_self));
    }
}

/// Sets the frequency of the TSC once calibrated
pub unsafe fn set_tsc_frequency(freq: NonZero<u64>) {
    unsafe {
        CPULocal::get_current_ptr().as_mut().tsc_frequency = freq;
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

    let jmp_to = unsafe {
        // the current CPU should take local 0
        *BOOT_CORE_ARGS.get() = MaybeUninit::new((process.clone(), idle_function));
        init_local_scheduler(process, idle_function)
    };

    for cpu in &cpus[1..] {
        cpu.goto_address.write(boot_cpu);
    }

    while READY_CPUS.load(core::sync::atomic::Ordering::Relaxed) != cpus.len() {
        core::hint::spin_loop();
    }

    jmp_to
}
