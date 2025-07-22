use core::sync::atomic::Ordering;

use crate::{
    arch::without_interrupts,
    scheduler::{SCHEDULER_INITED, cpu_context::Cid, this_process, this_thread},
    time, warn,
};

use crate::process::{Pid, ProcessInfo};

pub fn thread_exit(code: usize) -> ! {
    without_interrupts(|| {
        let current = super::this_thread();
        current.kill_thread(code);

        thread_yield();
        unreachable!("thread didn't exit")
    })
}

#[unsafe(no_mangle)]
pub fn thread_yield() {
    if !unsafe { *SCHEDULER_INITED.get() } {
        return;
    }

    unsafe {
        super::before_thread_yield();
    }
    crate::arch::threading::invoke_context_switch()
}

/// Sleeps the current thread for `ms` milliseconds.
/// unsafe because interrupts has to be disabled before calling this function
pub unsafe fn thread_sleep_for_ms(ms: u64) {
    #[cfg(debug_assertions)]
    let curr_time = time!(ms);

    let current = super::this_thread();
    current.sleep_for_ms(ms);
    thread_yield();
    assert!(curr_time + ms <= time!(ms), "thread didn't sleep");
}

/// Sleeps the current kernel thread for `ms` milliseconds.
/// safe because interrupts are properly managed in this function, expect interrupts to be enabled after calling this function
pub fn kthread_sleep_for_ms(ms: u64) {
    without_interrupts(|| unsafe { thread_sleep_for_ms(ms) })
}

pub fn try_cleanup_process(pid: Pid) -> Result<Option<usize>, ()> {
    let found_process =
        super::find(|process| process.pid() == pid, |process| process.clone()).ok_or(())?;
    if found_process.is_alive() {
        return Ok(None);
    }

    // process is dead
    // TODO: block multiple waits on same pid
    let Some(process_info) = super::remove(|p| p.pid() == pid) else {
        warn!("process with `{pid}` was already cleaned up by another operation");
        return Err(());
    };

    Ok(Some(process_info.exit_code))
}

/// waits for `pid` to exit
/// returns it's exit code after cleaning it up
pub fn wait_for_process(pid: Pid) -> Option<usize> {
    // cycles through the processes one by one until it finds the process with `pid`
    // returns the exit code of the process if it's a zombie and cleans it up
    let found_proc = super::find(|process| process.pid() == pid, |process| process.clone())?;

    let this = this_thread();
    this.wait_for_process(found_proc.clone());

    thread_yield();
    assert!(
        !found_proc.is_alive(),
        "Thread didn't wait for process to exit"
    );
    // process is dead
    // TODO: block multiple waits on same pid
    let Some(process_info) = super::remove(|p| p.pid() == pid) else {
        warn!("process with `{pid}` was already cleaned up by another wait operation");
        return None;
    };

    Some(process_info.exit_code)
}

/// Waits for thread with id `cid` to exit
/// threads don't have an exit code
pub fn wait_for_thread(cid: Cid) -> Option<()> {
    let this_thread = this_thread();
    let this_process = this_thread.process();
    let thread = this_process
        .threads
        .lock()
        .iter()
        .find(|thread| thread.cid() == cid)
        .cloned()?;

    this_thread.wait_for_thread(thread.clone());

    thread_yield();
    assert!(thread.is_dead(), "Thread didn't wait for thread to exit");

    Some(())
}

/// performs a WAIT for a futex to be unlocked
///
/// Waits for the value at `addr` to not be equal to `with_value`, returns true if the value was not equal to `with_value` at the time of the return
///
/// Doesn't wake up except when signaled by a WAKE operation and value isn't equal to `with_value` or if the timeout is reached.
///
/// # Safety
/// The caller must ensure that the address `addr` is valid and points to a valid futex.
pub unsafe fn wait_for_futex(addr: *mut u32, with_value: u32, timeout_ms: u64) -> bool {
    if unsafe { *addr != with_value } {
        return true;
    }

    let this_thread = this_thread();
    this_thread.wait_for_futex(addr, with_value, timeout_ms);

    thread_yield();
    unsafe { *addr != with_value }
}

/// Attempts to wake up `n` threads waiting on the futex at `target_addr`.
/// Returns the number of threads that were successfully woken up.
///
/// # Safety
/// Safe because addr is only accessed if any other thread is waiting on it.
pub fn wake_futex(addr: *mut u32, num_threads: usize) -> usize {
    if num_threads == 0 {
        return 0;
    }

    let this_process = this_process();
    this_process.wake_n_futexs(addr, num_threads)
}

#[unsafe(no_mangle)]
pub fn getinfo(pid: Pid) -> Option<ProcessInfo> {
    super::find(|p| p.pid() == pid, |t| ProcessInfo::from(&**t))
}

fn can_terminate(mut process_ppid: Pid, process_pid: Pid, terminator_pid: Pid) -> bool {
    if process_ppid == terminator_pid || process_pid == terminator_pid {
        return true;
    }

    while process_ppid != 0 {
        if process_ppid == terminator_pid {
            return true;
        }

        // find a parenty process and use it's ppid
        process_ppid =
            super::find(|p| p.pid() == process_ppid, |process| process.ppid()).unwrap_or(0);
    }

    false
}

fn terminate(process_pid: Pid, terminator_pid: Pid) {
    super::for_each(|process| {
        if process.pid() == process_pid {
            process.kill(1, Some(terminator_pid));
        }
    });

    // moves the parentership of all processes with `ppid` as `process_pid` to `terminator_pid`
    // prevents orphan processes from being left behind
    // TODO: figure out if orphan processes should be killed
    super::for_each(|p| {
        _ = p.ppid_atomic().compare_exchange(
            process_pid,
            terminator_pid,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    });
}

#[unsafe(no_mangle)]
/// can only Err if pid doesn't belong to process
pub fn pkill(pid: Pid) -> Result<(), ()> {
    let current_process = super::this_process();
    let current_pid = current_process.pid();

    let (process_ppid, process_pid) = super::find(
        |p| p.pid() == pid,
        |process| (process.ppid(), process.pid()),
    )
    .ok_or(())?;

    if can_terminate(process_ppid, process_pid, current_pid) {
        terminate(process_pid, current_pid);
        return Ok(());
    }
    Err(())
}
