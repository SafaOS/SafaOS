use core::{
    cell::SyncUnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::sync::Arc;
use limine::mp;

use crate::{
    arch::{self, registers::ArchCpuID, threading::restore_cpu_status, without_interrupts},
    limine::MP_RESPONSE,
    percpu::{self, CpuLocal},
    process::Process,
    scheduler::SCHEDULER,
};

unsafe extern "C" fn ap_boot(cpu: &mp::Cpu) -> ! {
    without_interrupts(|| {
        let cpu_local = unsafe { &*(cpu.extra.load(Ordering::Relaxed) as *const CpuLocal) };
        boot_cpu(cpu_local)
    })
}
/// The initial boot process
pub static INIT_PROCESS: SyncUnsafeCell<Option<Arc<Process>>> = SyncUnsafeCell::new(None);

/// The number of CPUs that have finished booting.
pub static READY_CPUS: AtomicUsize = AtomicUsize::new(1);

#[inline(always)]
fn boot_cpu(cpu: &'static CpuLocal) -> ! {
    arch::smp::init_cpu_with(cpu);

    cpu.online.store(true, Ordering::SeqCst);
    READY_CPUS.fetch_add(1, Ordering::Relaxed);

    unsafe {
        let idle_context = SCHEDULER
            .borrow()
            .idle_thread()
            .context_unchecked()
            .cpu_status()
            .as_ref();
        restore_cpu_status(idle_context)
    }
}

/// Initialize the CPUs.
pub fn init_cpus(proc: Arc<Process>) {
    unsafe {
        assert!((*INIT_PROCESS.get()).is_none(), "Double init");
        *INIT_PROCESS.get() = Some(proc);
    }

    let mp_res = &*MP_RESPONSE;
    let cpus = mp_res.cpus();

    let curr_id = ArchCpuID::get();

    for cpu in cpus {
        if ArchCpuID::from_cpu(cpu) == curr_id {
            continue;
        }

        let allocated = percpu::allocate_next(ArchCpuID::from_cpu(cpu));
        let go_to = ap_boot;

        cpu.extra
            .store(allocated as *const CpuLocal as u64, Ordering::Relaxed);
        cpu.goto_address.write(go_to);
    }

    let desire = cpus.len();

    while READY_CPUS.load(Ordering::Relaxed) < desire {
        core::hint::spin_loop();
    }

    percpu::init_bsp_all();
}
