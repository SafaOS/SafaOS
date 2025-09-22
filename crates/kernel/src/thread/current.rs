//! This module defines Functions and Operations related to the current thread.

use core::num::NonZero;
use core::sync::atomic::AtomicU32;

use crate::process::{self, Pid, WaitOnProcReason};
use crate::thread::Tid;
use crate::time;
use crate::{
    arch::without_interrupts,
    scheduler::{self, SCHEDULER_INITED},
    thread, warn,
};

/// Exit the current thread with the given exit code.
///
/// Exit codes are Process local and are used to indicate the reason for termination of a process and not a thread
/// if this thread is the last thread in the process, the process will be terminated with the given exit code, otherwise the exit code will be left unused.
pub fn exit(code: usize) -> ! {
    without_interrupts(|| {
        // current thread should be dropped at the end of this
        unsafe {
            let current = thread::current();
            current.kill(code);
        }
        self::yield_now();
        unreachable!("thread didn't exit")
    })
}

/// Sleeps the current thread for `ms` milliseconds.
pub fn sleep_for_ms(ms: u64) {
    let Some(ms) = NonZero::new(ms) else {
        return;
    };

    without_interrupts(|| {
        let curr_time = time!(ms);

        let current = thread::current();
        current.sleep_for_ms(ms);
        yield_now();
        assert!(
            ms.saturating_add(curr_time).get() <= time!(ms),
            "thread didn't sleep"
        );
    });
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
pub fn wait_for_process(pid: Pid) -> Option<usize> {
    // cycles through the processes one by one until it finds the process with `pid`
    // returns the exit code of the process if it's a zombie and cleans it up
    let found_proc =
        scheduler::process_list::find(|process| process.pid() == pid, |process| process.clone())?;

    if !found_proc.is_alive() {
        let Some(process_info) = scheduler::process_list::remove(|p| p.pid() == pid) else {
            warn!("process with `{pid}` was already cleaned up by another wait operation");
            return None;
        };

        return process_info.exit_code;
    }

    let this_thread = thread::current();
    without_interrupts(|| {
        found_proc.sleep_thread(this_thread, WaitOnProcReason::WaitingOnSelf, None);
        self::yield_now();
    });

    assert!(
        !found_proc.is_alive(),
        "Thread didn't wait for process to exit"
    );
    // process is dead
    // TODO: block multiple waits on same pid
    let Some(process_info) = scheduler::process_list::remove(|p| p.pid() == pid) else {
        warn!("process with `{pid}` was already cleaned up by another wait operation");
        return None;
    };

    Some(
        process_info
            .exit_code
            .expect("process dead but exit code hasn't been set"),
    )
}

/// Sleeps the current thread until the thread with tid `tid` exits.
// NOTE: threads don't have an exit code
pub fn wait_for_thread(tid: Tid) -> Option<()> {
    let this_thread = thread::current();
    let this_process = this_thread.process();
    let thread = this_process
        .threads
        .lock()
        .iter()
        .find(|thread| thread.tid() == tid && !thread.is_dead())
        .cloned()?;

    without_interrupts(|| {
        this_process.sleep_thread(
            this_thread.clone(),
            WaitOnProcReason::WaitingOnChild(thread.clone()),
            None,
        );
        self::yield_now();
    });
    assert!(thread.is_dead(), "Thread didn't wait for thread to exit");

    Some(())
}

/// performs a WAIT for a futex to be unlocked
///
/// Waits for the value at `addr` to not be equal to `with_value`, returns true if the value was not equal to `with_value` at the time of the return
///
/// Doesn't wake up except when signaled by a WAKE (see [`crate::process::current::wake_futex`]) operation and value isn't equal to `with_value` or if the timeout is reached.
///
/// # Safety
/// The caller must ensure that the address `addr` is valid and points to a valid futex.
pub unsafe fn wait_for_futex(addr: &AtomicU32, with_value: u32, timeout_ms: u64) -> bool {
    let Some(timeout_ms) = NonZero::new(timeout_ms) else {
        return false;
    };
    let duration = if timeout_ms.get() == u64::MAX {
        None
    } else {
        Some(timeout_ms)
    };

    let this_thread = thread::current();
    let this_proc = process::current();

    if addr.load(core::sync::atomic::Ordering::SeqCst) != with_value {
        return true;
    }

    let timeouted = without_interrupts(move || {
        let timeout_at = this_proc.sleep_thread(
            this_thread,
            WaitOnProcReason::WaitingOnFutex(addr),
            duration,
        );
        self::yield_now();

        if let Some(timeout_at) = timeout_at {
            time!(ms) >= timeout_at.get()
        } else {
            false
        }
    });
    !timeouted
}
