use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use crate::{
    VirtAddr,
    arch::without_interrupts,
    memory::paging::MapToError,
    threading::{
        SCHEDULER_INITED,
        cpu_context::{Cid, ContextPriority, Thread},
        this_process, this_thread,
    },
    time,
    utils::types::Name,
    warn,
};
use alloc::boxed::Box;
use bitflags::bitflags;
use safa_utils::{
    abi::raw::{self, processes::ProcessStdio},
    make_path,
    path::PathBuf,
};
use thiserror::Error;

use crate::{
    drivers::vfs::{FSError, FSObjectType, FSResult, VFS_STRUCT, expose::File},
    utils::{
        elf::{Elf, ElfError},
        errors::ErrorStatus,
        io::Readable,
        path::Path,
    },
};

use super::{
    Pid,
    process::{Process, ProcessInfo},
};

#[unsafe(no_mangle)]
pub fn process_exit(code: usize) -> ! {
    without_interrupts(|| {
        let current_process = super::this_process();
        current_process.kill(code, None);

        thread_yield();
        unreachable!("process didn't exit")
    })
}

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

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct SpawnFlags: u8 {
        const CLONE_RESOURCES = 1 << 0;
        const CLONE_CWD = 1 << 1;
    }
}

impl From<raw::processes::SpawnFlags> for SpawnFlags {
    fn from(value: raw::processes::SpawnFlags) -> Self {
        unsafe { Self::from_bits_retain(core::mem::transmute(value)) }
    }
}

#[derive(Debug, Clone, Error)]

pub enum SpawnError {
    #[error("out of memory")]
    MapToError(#[from] MapToError),
    #[error("failed to map elf to memory {0}")]
    ElfError(#[from] ElfError),
    #[error("error while creating process {0}")]
    FSError(#[from] FSError),
}

#[inline(always)]
fn spawn_inner(
    name: Name,
    flags: SpawnFlags,
    stdio: ProcessStdio,
    create_process: impl FnOnce(
        Name,
        Pid,
        Pid,
        Box<PathBuf>,
    ) -> Result<(Arc<Process>, Arc<Thread>), SpawnError>,
) -> Result<Pid, SpawnError> {
    let this_process = this_process();
    let this_state = this_process.state();

    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        this_state.cwd()
    } else {
        make_path!("ram", "")
    };

    let current_process = super::this_process();
    let current_pid = current_process.pid();
    let new_pid = super::SCHEDULER.write().add_pid();

    let cwd = Box::new(cwd.into_owned().unwrap());
    let (process, root_thread) = create_process(name, current_pid, new_pid, cwd)?;

    let provide_resources = || {
        let mut state = process.state_mut();
        let Some(process_resources) = state.resource_manager_mut() else {
            unreachable!();
        };

        drop(this_state);
        let mut this_state = this_process.state_mut();

        let clone = if flags.contains(SpawnFlags::CLONE_RESOURCES) {
            this_state.clone_resources()
        } else {
            // clone only necessary resources
            let mut resources = heapless::Vec::<usize, 3>::new();
            if let Some(stdin) = stdio.stdin.into() {
                _ = resources.push(stdin);
            }

            if let Some(stdout) = stdio.stdout.into() {
                _ = resources.push(stdout);
            }

            if let Some(stderr) = stdio.stderr.into() {
                _ = resources.push(stderr);
            }

            if resources.is_empty() {
                return Ok(());
            }
            this_state.clone_specific_resources(&resources)?
        };

        process_resources.overwrite_resources(clone);
        Ok(())
    };

    provide_resources().map_err(|()| FSError::InvalidResource)?;

    let pid = super::add(process, root_thread);
    Ok(pid)
}

fn spawn<T: Readable>(
    name: Name,
    reader: &T,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    priority: ContextPriority,
    stdio: ProcessStdio,
    custom_stack_size: Option<usize>,
) -> Result<Pid, SpawnError> {
    spawn_inner(name, flags, stdio, |name: Name, ppid, pid, cwd| {
        let elf = Elf::new(reader)?;
        let process = Process::from_elf(
            name,
            pid,
            ppid,
            cwd,
            elf,
            argv,
            env,
            priority,
            stdio,
            custom_stack_size,
        )?;
        Ok(process)
    })
}

/// spawns an elf process from a path
pub fn pspawn(
    name: Name,
    path: Path,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    priority: ContextPriority,
    stdio: ProcessStdio,
    custom_stack_size: Option<usize>,
) -> Result<Pid, FSError> {
    let file = File::open(path)?;

    if file.kind() != FSObjectType::File {
        return Err(FSError::NotAFile);
    }

    spawn(
        name,
        &file,
        argv,
        env,
        flags,
        priority,
        stdio,
        custom_stack_size,
    )
    .map_err(|_| FSError::NotExecutable)
}

/// Spawns a userspace function in a new thread
///
/// if `cpu` is Some it will append to that CPU instead of choosing one, use CPU 0 to append to boot CPU
pub fn thread_spawn(
    entry_point: VirtAddr,
    argument_ptr: VirtAddr,
    priority: Option<ContextPriority>,
    cpu: Option<usize>,
    custom_stack_size: Option<usize>,
) -> Result<Cid, MapToError> {
    let this = this_process();
    let (thread, cid) = Process::add_thread_to_process(
        &this,
        entry_point,
        argument_ptr,
        priority,
        custom_stack_size,
    )?;
    super::add_thread(thread, cpu);
    Ok(cid)
}

/// Spawns a kernel function in a new thread
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

/// also ensures the cwd ends with /
/// will only Err if new_dir doesn't exists or is not a directory
#[unsafe(no_mangle)]
pub fn chdir(new_dir: Path) -> FSResult<()> {
    VFS_STRUCT.read().verify_path_dir(new_dir)?;

    let process = this_process();
    let mut state = process.state_mut();
    let cwd = state.cwd_mut();

    if new_dir.is_absolute() {
        *cwd = new_dir.into_owned_simple()?;
    } else {
        cwd.append_simplified(new_dir)?;
    }

    Ok(())
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

#[unsafe(no_mangle)]
/// extends program break by `amount`
/// returns the new program break ptr
/// on fail returns null
pub fn sbrk(amount: isize) -> Result<*mut u8, ErrorStatus> {
    let current_process = super::this_process();
    let mut state = current_process.state_mut();
    state.extend_data_by(amount).ok_or(ErrorStatus::OutOfMemory)
}
