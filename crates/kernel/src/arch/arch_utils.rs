use core::{
    cell::SyncUnsafeCell,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

use crate::debug;

/// Instructs all parked CPUs to execute a function.
pub fn parked_cpus_do(f: extern "C" fn(usize), arg: usize) {
    PARKED_CORES_RESPONSE_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
    unsafe {
        *PARKED_CORES_ARGS.get() = arg;
    }

    let expected_core_count = PARKED_CORES_COUNT.load(Ordering::Relaxed);

    debug!("calling function: {f:?}({arg:#x}) on {expected_core_count} CPU(s)");
    PARKED_CORES_GOTO.store(f as *mut (), core::sync::atomic::Ordering::Release);

    while PARKED_CORES_RESPONSE_COUNT.load(Ordering::Relaxed) < expected_core_count {
        core::hint::spin_loop();
    }

    debug!("CPU(s) finished");
    PARKED_CORES_GOTO.store(core::ptr::null_mut(), core::sync::atomic::Ordering::Release);
}

static PARKED_CORES_GOTO: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static PARKED_CORES_ARGS: SyncUnsafeCell<usize> = SyncUnsafeCell::new(0);
static PARKED_CORES_RESPONSE_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static PARKED_CORES_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Parks the current CPU and makes it available for other CPUs to execute a function on.
pub fn cpu_park() -> ! {
    PARKED_CORES_COUNT.fetch_add(1, Ordering::Relaxed);

    loop {
        let goto_addr = PARKED_CORES_GOTO.load(Ordering::Acquire);
        if !goto_addr.is_null() {
            let args = unsafe { *PARKED_CORES_ARGS.get() };
            unsafe {
                let fn_ptr: extern "C" fn(usize) = core::mem::transmute(goto_addr);
                fn_ptr(args);
            }

            debug!("a CPU was done");

            PARKED_CORES_RESPONSE_COUNT.fetch_add(1, Ordering::Relaxed);
            while !PARKED_CORES_GOTO.load(Ordering::Acquire).is_null() {
                core::hint::spin_loop();
            }
        }
        core::hint::spin_loop();
    }
}
