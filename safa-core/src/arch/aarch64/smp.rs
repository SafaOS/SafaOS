use core::{
    arch::asm, cell::SyncUnsafeCell, mem::MaybeUninit, ptr::NonNull, sync::atomic::AtomicUsize,
};

use alloc::sync::Arc;
use limine::mp::Cpu;

use crate::{
    VirtAddr,
    arch::{
        aarch64::registers::MPIDR,
        paging::{CURRENT_HIGHER_HALF_TABLE, set_current_higher_page_table_phys},
        smp_misc,
        threading::{CPUStatus, restore_cpu_status},
        without_interrupts,
    },
    debug,
    limine::MP_RESPONSE,
    process::Process,
    scheduler::Scheduler,
    thread::ContextPriority,
};

#[repr(transparent)]
#[derive(Debug)]
pub struct CPULocal(Option<Scheduler>);
impl CPULocal {
    pub fn on_allocated(&mut self) {
        // Does nothing on aarch64
        _ = self;
    }

    fn allocate() -> &'static mut Self {
        smp_misc::allocate_cpu_local(Self(None))
    }

    /// Returns the scheduler
    #[inline(always)]
    pub fn scheduler(&self) -> Option<&Scheduler> {
        self.0.as_ref()
    }

    /// Gets a ptr to the current CPU local
    #[inline]
    pub unsafe fn get_current_ptr() -> NonNull<Self> {
        let ptr: *mut Self;
        unsafe { asm!("mrs {}, tpidr_el1", out(reg) ptr, options(nostack, nomem)) }
        unsafe { NonNull::new_unchecked(ptr) }
    }

    /// Gets a reference to the current CPU local
    ///
    /// Safety: should succeed as long as we are past the first ever arch init, so it is marked safe
    #[inline]
    pub fn get_current() -> &'static Self {
        unsafe { Self::get_current_ptr().as_ref() }
    }
}

fn set_scheduler(schd: Scheduler) {
    unsafe {
        let mut ptr = CPULocal::get_current_ptr();
        let r = ptr.as_mut();
        assert!(r.0.is_none(), "Scheduler already initialized");
        let schd_mut = r.0.insert(schd);
        schd_mut.idle_thread().set_scheduler(schd_mut)
    }
}

unsafe fn set_tpidr(value: VirtAddr) {
    crate::serial!("tpidr_el1 set to: {value:#x}\n");
    unsafe {
        asm!("msr tpidr_el1, {}", in(reg) value.into_raw(), options(nomem, nostack));
    }
}

/// Creates a cpu local storage from a given process and an idle function
/// creates and adds a thread to the given process that is the idle thread for the caller CPU
///
/// unsafe because the caller is responsible for the memory which was allocated using a Box
unsafe fn init_local_scheduler(
    process: &Arc<Process>,
    idle_function: fn() -> !,
) -> NonNull<CPUStatus> {
    let (thread, _) = process
        .threads_manager()
        .create_thread(
            process,
            VirtAddr::from(idle_function as usize),
            VirtAddr::null(),
            Some(ContextPriority::Low),
            None,
        )
        .expect("Failed to allocate IDLE Thread");

    let status = unsafe { thread.context_unchecked().cpu_status() };

    let scheduler = Scheduler::new(thread);
    set_scheduler(scheduler);

    status
}

fn boot_core_inner(process: &Arc<Process>, idle_function: fn() -> !) -> ! {
    let cpuid = MPIDR::read().cpuid();
    unsafe {
        debug!("setting up CPU: {}", cpuid);

        let status = init_local_scheduler(process, idle_function);
        let status = status.as_ref();

        debug!(
            "CPU {}: jumping to {:#x}, with stack at {:#x}",
            cpuid,
            status.at(),
            status.stack_at()
        );
        READY_CPUS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        restore_cpu_status(status)
    }
}

extern "C" fn boot_cpu(_: &Cpu) -> ! {
    without_interrupts(|| {
        super::setup_cpu_basics();

        unsafe {
            let ttbr1_el1 = *CURRENT_HIGHER_HALF_TABLE.get();
            set_current_higher_page_table_phys(ttbr1_el1);

            super::setup_cpu_mp();
            super::setup_cpu_pherphials();

            let (process, idle_function) = (*BOOT_CORE_ARGS.get()).assume_init_ref();
            boot_core_inner(process, *idle_function)
        }
    })
}

pub(super) fn init_cpu_local() {
    unsafe {
        let cpu_local = CPULocal::allocate();
        set_tpidr(VirtAddr::from_ptr(cpu_local));
    }
}

static BOOT_CORE_ARGS: SyncUnsafeCell<MaybeUninit<(Arc<Process>, fn() -> !)>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());
pub static READY_CPUS: AtomicUsize = AtomicUsize::new(1);

pub unsafe fn init_cpus(process: &Arc<Process>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let cpus = (*MP_RESPONSE).cpus();
    smp_misc::reserve_cpus(cpus.len() - 1);

    let jmp_to = unsafe {
        // the current CPU should take local 0
        *BOOT_CORE_ARGS.get() = MaybeUninit::new((process.clone(), idle_function));
        init_local_scheduler(process, idle_function)
    };

    for cpu in cpus {
        if MPIDR::from_bits(cpu.mpidr).cpuid() != MPIDR::read().cpuid() {
            cpu.goto_address.write(boot_cpu);
        }
    }

    while READY_CPUS.load(core::sync::atomic::Ordering::Relaxed) != cpus.len() {
        core::hint::spin_loop();
    }

    jmp_to
}
