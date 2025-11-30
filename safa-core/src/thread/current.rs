//! This module defines Functions and Operations related to the current thread.

use core::num::NonZero;
use core::sync::atomic::AtomicU32;

use crate::process::{self, Pid, WaitOnProcReason};
use crate::scheduler::wait_queue::WaitError;
use crate::thread::Tid;
use crate::{
    arch::without_interrupts,
    scheduler::{self, SCHEDULER_INITED},
    thread, warn,
};

/// Exit the current thread with the given exit code.
///
/// Exit codes are Process local and are used to indicate the reason for termination of a process and not a thread
/// if this thread is the last thread in the process, the process will be terminated with the given exit code, otherwise the exit code will be left unused.
pub fn exit(code: isize) -> ! {
    without_interrupts(|| {
        // current thread should be dropped at the end of this
        unsafe { thread::with_current(|curr| curr.kill(code)) }
        self::yield_now();
        unreachable!("thread didn't exit")
    })
}

/// Sleeps the current thread for `ms` milliseconds.
pub fn sleep_for_ms(ms: u64) -> Result<(), WaitError> {
    let Some(ms) = NonZero::new(ms) else {
        return Ok(());
    };

    without_interrupts(|| {
        thread::with_current(|current| {
            if unsafe { current.prepare_sleep_for_ms(ms) } {
                yield_now();
            }

            if current.should_terminate() {
                Err(WaitError::ForceTerminated)
            } else {
                assert!(
                    unsafe { current.operation_timeout() },
                    "thread didn't sleep, status: {:#x?}",
                    &*current.status_mut()
                );
                Ok(())
            }
        })
    })
}

/// Yields execution to the next thread that is ready to run, in the thread queue for the current CPU.
pub fn yield_now() {
    without_interrupts(|| {
        if !unsafe { *SCHEDULER_INITED.get() } {
            return;
        }

        unsafe {
            crate::scheduler::before_thread_yield();
        }
        crate::arch::threading::invoke_context_switch()
    });
}

/// Sleeps the current thread until the process with `pid` exits.
/// Returns the exit code of the process after cleaning it up.
pub fn wait_for_process(pid: Pid) -> Result<Option<isize>, WaitError> {
    // cycles through the processes one by one until it finds the process with `pid`
    // returns the exit code of the process if it's a zombie and cleans it up
    let Some(found_proc) =
        scheduler::process_list::find(|process| process.pid() == pid, |process| process.clone())
    else {
        return Ok(None);
    };

    if !found_proc.is_alive() {
        let Some(process_info) = scheduler::process_list::remove(|p| p.pid() == pid) else {
            warn!("process with `{pid}` was already cleaned up by another wait operation");
            return Ok(None);
        };

        return Ok(process_info.exit_code);
    }

    found_proc.sleep_thread(WaitOnProcReason::WaitingOnSelf, None)?;
    assert!(
        !found_proc.is_alive(),
        "Thread didn't wait for process to exit"
    );
    // process is dead
    // TODO: block multiple waits on same pid
    let Some(process_info) = scheduler::process_list::remove(|p| p.pid() == pid) else {
        warn!("process with `{pid}` was already cleaned up by another wait operation");
        return Ok(None);
    };

    Ok(Some(
        process_info
            .exit_code
            .expect("process dead but exit code hasn't been set"),
    ))
}

/// Sleeps the current thread until the thread with tid `tid` exits.
// NOTE: threads don't have an exit code
//
// returns true if the thread was awaited false if it wasn't
pub fn wait_for_thread(tid: Tid) -> Result<bool, WaitError> {
    thread::with_current(|this_thread| {
        let this_process = this_thread.process();
        let try_remove = this_process
            .threads_manager()
            .remove_tid(tid)
            .map_err(|e| e.clone());

        // FIXME: This is shit, it doesn't always remove thread IDs, we can store thread IDs better anyways using a wrapping counter.
        match try_remove {
            Ok(false) => Ok(false),
            Err(thread) if !thread.is_dead() => {
                this_process
                    .sleep_thread(WaitOnProcReason::WaitingOnChild(thread.clone()), None)?;
                assert!(thread.is_dead(), "Thread didn't wait for thread to exit");

                Ok(true)
            }
            Ok(true) | Err(_) => Ok(true),
        }
    })
}

/// performs a WAIT for a futex to be unlocked
///
/// Waits for the value at `addr` to not be equal to `with_value`, returns true if the value was not equal to `with_value` at the time of the return
///
/// Doesn't wake up except when signaled by a WAKE (see [`crate::process::current::wake_futex`]) operation and value isn't equal to `with_value` or if the timeout is reached.
///
/// # Safety
/// The caller must ensure that the address `addr` is valid and points to a valid futex.
pub unsafe fn wait_for_futex(
    addr: &AtomicU32,
    with_value: u32,
    timeout_ms: u64,
) -> Result<(), WaitError> {
    let Some(timeout_ms) = NonZero::new(timeout_ms) else {
        return Err(WaitError::Timeout);
    };
    let duration = if timeout_ms.get() == u64::MAX {
        None
    } else {
        Some(timeout_ms)
    };

    let this_proc = process::current();

    if addr.load(core::sync::atomic::Ordering::SeqCst) != with_value {
        return Ok(());
    }

    this_proc.sleep_thread(WaitOnProcReason::WaitingOnFutex(addr, with_value), duration)
}
