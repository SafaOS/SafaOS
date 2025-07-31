//! This module contains functions related to the current process.

use core::num::NonZero;
use core::sync::atomic::AtomicU32;

use crate::arch::without_interrupts;
use crate::memory::paging::MapToError;
use crate::process::{Pid, Process};
use crate::thread::{self, Tid};
use crate::{VirtAddr, process, scheduler, warn};

pub fn exit(code: usize) -> ! {
    without_interrupts(|| {
        let current_process = process::current();
        current_process.kill(code, None);

        thread::current::yield_now();
        unreachable!("process didn't exit")
    })
}

/// Spawns a userspace function in a new thread in the current process.
///
/// if `cpu` is Some it will append to that CPU instead of choosing one, use CPU 0 to append to boot CPU
pub fn thread_spawn(
    entry_point: VirtAddr,
    argument_ptr: VirtAddr,
    priority: Option<ContextPriority>,
    cpu: Option<usize>,
    custom_stack_size: Option<NonZero<usize>>,
) -> Result<Tid, MapToError> {
    let this = process::current();
    let (thread, cid) = Process::add_thread_to_process(
        &this,
        entry_point,
        argument_ptr,
        priority,
        custom_stack_size,
    )?;
    scheduler::add_thread(thread, cpu);
    Ok(cid)
}

/// Spawns a kernel function in a new thread in the current process.
///
/// if `cpu` is Some it will append to that CPU instead of choosing one, use CPU 0 to append to boot CPU
pub fn kernel_thread_spawn<T: 'static>(
    func: fn(tid: Tid, &'static T) -> !,
    arg: &'static T,
    priority: Option<ContextPriority>,
    cpu: Option<usize>,
) -> Result<Tid, MapToError> {
    thread_spawn(
        VirtAddr::from(func as usize),
        VirtAddr::from(arg as *const T as usize),
        priority,
        cpu,
        None,
    )
}

/// Attempts to cleanup a process that is a child of the current process with the given pid, collecting it's exit status.
///
/// # Returns:
/// - Ok(None) if the current process has parentship of the process, and the process is still alive.
/// - Ok(Some(exit_code)) if the process is dead, returns the exit code.
/// - Err([ErrorStatus::MissingPermissions]) if the current process does not have parentship of the process.
/// - Err([ErrorStatus::InvalidPid]) if the process with the given pid does not exist.
pub fn try_cleanup_process(pid: Pid) -> Result<Option<usize>, ErrorStatus> {
    let found_process = scheduler::find(|process| process.pid() == pid, |process| process.clone())
        .ok_or(ErrorStatus::InvalidPid)?;

    if found_process.ppid() != process::current_pid() {
        return Err(ErrorStatus::MissingPermissions);
    }

    if found_process.is_alive() {
        return Ok(None);
    }

    // process is dead
    // TODO: block multiple waits on same pid
    let Some(process_info) = scheduler::remove(|p| p.pid() == pid) else {
        warn!("process with `{pid}` was already cleaned up by another operation");
        return Err(ErrorStatus::InvalidPid);
    };

    Ok(Some(process_info.exit_code))
}

/// Attempts to wake up `n` threads waiting on the futex at `target_addr` in the current process.
/// Returns the number of threads that were successfully woken up.
///
/// # Safety
/// Safe because addr is only accessed if any other thread is waiting on it and has dereferenced it previously.
pub fn wake_futex(addr: *const AtomicU32, num_threads: usize) -> usize {
    if num_threads == 0 {
        return 0;
    }

    let this_process = process::current();
    let n = this_process.wake_n_futexs(addr, num_threads);
    n
}

use crate::thread::ContextPriority;
use safa_abi::errors::ErrorStatus;

/// extends program break by `amount`
/// returns the new program break ptr
/// on fail returns null
pub fn extend_data_break(amount: isize) -> Result<*mut u8, ErrorStatus> {
    let current_process = process::current();
    let mut state = current_process.state_mut();
    state.extend_data_by(amount).ok_or(ErrorStatus::OutOfMemory)
}

use crate::drivers::vfs::{FSResult, VFS_STRUCT};
use crate::utils::path::Path;

/// also ensures the cwd ends with /
/// will only Err if new_dir doesn't exists or is not a directory
pub fn chdir(new_dir: Path) -> FSResult<()> {
    VFS_STRUCT.read().verify_path_dir(new_dir)?;

    let process = process::current();
    let mut state = process.state_mut();
    let cwd = state.cwd_mut();

    if new_dir.is_absolute() {
        *cwd = new_dir.into_owned_simple()?;
    } else {
        cwd.append_simplified(new_dir)?;
    }

    Ok(())
}
