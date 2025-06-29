use core::sync::atomic::Ordering;

use crate::{
    VirtAddr,
    arch::{disable_interrupts, enable_interrupts, threading::CPUStatus},
    memory::paging::{MapToError, PhysPageTable},
    threading::{cpu_context::Cid, this},
    time,
    utils::types::Name,
};
use alloc::boxed::Box;
use bitflags::bitflags;
use safa_utils::{
    abi::raw::{self, processes::AbiStructures},
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
    task::{Task, TaskInfo},
};

#[unsafe(no_mangle)]
pub fn task_exit(code: usize) -> ! {
    let current = super::current();
    current.kill(code, None);
    drop(current);

    thread_yield();
    unreachable!("task didn't exit")
}

pub fn thread_exit(code: usize) -> ! {
    let current = super::current();
    current.kill_current_thread(code);
    drop(current);

    thread_yield();
    unreachable!("thread didn't exit ")
}

#[unsafe(no_mangle)]
pub fn thread_yield() {
    crate::arch::threading::invoke_context_switch()
}

/// Sleeps the current thread for `ms` milliseconds.
/// unsafe because interrupts has to be disabled before calling this function
pub unsafe fn thread_sleep_for_ms(ms: u64) {
    #[cfg(debug_assertions)]
    let curr_time = time!(ms);

    let current = super::current();
    unsafe { current.context_sleep_for_ms(ms) };
    thread_yield();
    // makes sure the thread slept for the correct amount of time
    #[cfg(debug_assertions)]
    debug_assert!(
        curr_time + ms <= time!(ms),
        "Thread didn't sleep for the correct amount of time, only waited for: {}ms",
        curr_time + ms - time!(ms)
    );
}

/// Sleeps the current kernel thread for `ms` milliseconds.
/// safe because interrupts are properly managed in this function, expect interrupts to be enabled after calling this function
pub fn kthread_sleep_for_ms(ms: u64) {
    unsafe {
        disable_interrupts();
        thread_sleep_for_ms(ms);
        enable_interrupts();
    };
}

#[unsafe(no_mangle)]
/// waits for `pid` to exit
/// returns it's exit code after cleaning it up
pub fn wait(pid: Pid) -> usize {
    // loops through the processes until it finds the process with `pid` as a zombie
    loop {
        // cycles through the processes one by one until it finds the process with `pid`
        // returns the exit code of the process if it's a zombie and cleans it up
        // if it's not a zombie it will be caught by the next above loop
        let found = super::find(|process| process.pid() == pid);
        let found = found.map(|process| process.state().exit_code());

        return match found {
            Some(Some(exit_code)) => {
                // cleans up the process
                super::remove(|p| p.pid() == pid);
                exit_code
            }
            Some(None) => {
                thread_yield();
                continue;
            }
            None => 0,
        };
    }
}

#[unsafe(no_mangle)]
pub fn getinfo(pid: Pid) -> Option<TaskInfo> {
    let found = super::find(|p| p.pid() == pid);
    found.map(|p| TaskInfo::from(&*p))
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
    structures: AbiStructures,
    create_task: impl FnOnce(Name, Pid, Box<PathBuf>) -> Result<Task, SpawnError>,
) -> Result<Pid, SpawnError> {
    let this_task = this().state();
    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        this_task.cwd()
    } else {
        make_path!("ram", "")
    };

    let current = super::current();
    let current_pid = current.pid();

    let cwd = Box::new(cwd.into_owned().unwrap());
    let task = create_task(name, current_pid, cwd)?;

    let provide_resources = || {
        let mut state = task.state_mut();
        let Some(task_resources) = state.resource_manager_mut() else {
            unreachable!();
        };

        drop(this_task);
        let mut this_task = this().state_mut();

        let clone = if flags.contains(SpawnFlags::CLONE_RESOURCES) {
            this_task.clone_resources()
        } else {
            // clone only necessary resources
            let mut resources = heapless::Vec::<usize, 3>::new();
            if let Some(stdin) = structures.stdio.stdin.into() {
                _ = resources.push(stdin);
            }

            if let Some(stdout) = structures.stdio.stdout.into() {
                _ = resources.push(stdout);
            }

            if let Some(stderr) = structures.stdio.stderr.into() {
                _ = resources.push(stderr);
            }

            if resources.is_empty() {
                return Ok(());
            }
            this_task.clone_specific_resources(&resources)?
        };

        task_resources.overwrite_resources(clone);
        Ok(())
    };

    provide_resources().map_err(|()| FSError::InvalidResource)?;

    let pid = super::add(task);
    Ok(pid)
}

// used by tests...
#[allow(unused)]
pub fn function_spawn(
    name: Name,
    function: fn() -> !,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    structures: AbiStructures,
) -> Result<Pid, SpawnError> {
    spawn_inner(name, flags, structures, |name: Name, ppid, cwd| {
        let mut page_table = PhysPageTable::create()?;
        let context = unsafe {
            CPUStatus::create_root(
                &mut page_table,
                argv,
                env,
                structures,
                VirtAddr::from(function as usize),
                false,
            )
        }?;

        let task = Task::new(
            name,
            0,
            ppid,
            cwd,
            page_table,
            context,
            VirtAddr::null(),
            false,
        );
        Ok(task)
    })
}

fn spawn<T: Readable>(
    name: Name,
    reader: &T,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    structures: AbiStructures,
) -> Result<Pid, SpawnError> {
    spawn_inner(name, flags, structures, |name: Name, ppid, cwd| {
        let elf = Elf::new(reader)?;
        let task = Task::from_elf(name, 0, ppid, cwd, elf, argv, env, structures)?;
        Ok(task)
    })
}

/// spawns an elf process from a path
pub fn pspawn(
    name: Name,
    path: Path,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    structures: AbiStructures,
) -> Result<Pid, FSError> {
    let file = File::open(path)?;

    if file.kind() != FSObjectType::File {
        return Err(FSError::NotAFile);
    }

    spawn(name, &file, argv, env, flags, structures).map_err(|_| FSError::NotExecutable)
}

pub fn thread_spawn(entry_point: VirtAddr, argument_ptr: VirtAddr) -> Result<Cid, MapToError> {
    let this = this();
    this.append_context(entry_point, argument_ptr)
}

pub fn kernel_thread_spawn<T: 'static>(
    func: fn(cid: Cid, &'static T) -> !,
    arg: &'static T,
) -> Result<Cid, MapToError> {
    thread_spawn(
        VirtAddr::from(func as usize),
        VirtAddr::from(arg as *const T as usize),
    )
}

/// also ensures the cwd ends with /
/// will only Err if new_dir doesn't exists or is not a directory
#[unsafe(no_mangle)]
pub fn chdir(new_dir: Path) -> FSResult<()> {
    VFS_STRUCT.read().verify_path_dir(new_dir)?;

    let mut state = this().state_mut();
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

        let pprocess = super::find(|p| p.pid() == process_ppid);
        process_ppid = pprocess.map(|process| process.ppid()).unwrap_or(0);
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
    let current = super::current();
    let current_pid = current.pid();

    let (process_ppid, process_pid) = super::find(|p| p.pid() == pid)
        .map(|process| (process.ppid(), process.pid()))
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
    let current = super::current();
    let mut state = current.state_mut();
    state.extend_data_by(amount).ok_or(ErrorStatus::OutOfMemory)
}
