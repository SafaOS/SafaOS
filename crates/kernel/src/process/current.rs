//! This module contains functions related to the current process.

use crate::arch::without_interrupts;
use crate::memory::paging::MapToError;
use crate::process::Process;
use crate::scheduler::cpu_context::Cid;
use crate::scheduler::expose::thread_yield;
use crate::{VirtAddr, process, scheduler};

pub fn exit(code: usize) -> ! {
    without_interrupts(|| {
        let current_process = process::current();
        current_process.kill(code, None);

        thread_yield();
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
    custom_stack_size: Option<usize>,
) -> Result<Cid, MapToError> {
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
    func: fn(cid: Cid, &'static T) -> !,
    arg: &'static T,
    priority: Option<ContextPriority>,
    cpu: Option<usize>,
) -> Result<Cid, MapToError> {
    thread_spawn(
        VirtAddr::from(func as usize),
        VirtAddr::from(arg as *const T as usize),
        priority,
        cpu,
        None,
    )
}

use safa_abi::errors::ErrorStatus;
use safa_abi::raw::processes::ContextPriority;

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
