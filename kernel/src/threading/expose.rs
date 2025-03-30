use core::{arch::asm, sync::atomic::Ordering};

use crate::{
    arch::threading::CPUStatus,
    memory::paging::{MapToError, PhysPageTable},
    utils::types::Name,
};
use alloc::boxed::Box;
use bitflags::bitflags;
use safa_utils::make_path;

use crate::{
    drivers::vfs::{expose::File, FSError, FSResult, InodeType, VFS_STRUCT},
    khalt,
    utils::{
        elf::{Elf, ElfError},
        errors::ErrorStatus,
        io::Readable,
        path::Path,
    },
};

use super::{
    task::{Task, TaskInfo, TaskState},
    this_state, this_state_mut, Pid,
};

#[no_mangle]
pub fn thread_exit(code: usize) -> ! {
    let current = super::current();
    current.kill(code, None);
    drop(current);

    // enables interrupts if they were disabled to give control back to the scheduler
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!("sti")
    }
    khalt()
}

#[no_mangle]
pub fn thread_yeild() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!("int 0x20")
    }
}

#[no_mangle]
/// waits for `pid` to exit
/// returns it's exit code after cleaning it up
pub fn wait(pid: usize) -> usize {
    // loops through the processes until it finds the process with `pid` as a zombie
    loop {
        // cycles through the processes one by one untils it finds the process with `pid`
        // returns the exit code of the process if it's a zombie and cleans it up
        // if it's not a zombie it will be caught by the next above loop
        let found = super::find(|process| process.pid == pid);
        let found = found.map(|process| process.state().map(|state| state.exit_code()).flatten());

        return match found {
            Some(Some(exit_code)) => {
                // cleans up the process
                super::remove(|p| p.pid == pid);
                exit_code
            }
            Some(None) => {
                thread_yeild();
                continue;
            }
            None => 0,
        };
    }
}

#[no_mangle]
pub fn getinfo(pid: Pid) -> Option<TaskInfo> {
    let found = super::find(|p| p.pid == pid);
    found.map(|p| TaskInfo::from(&*p))
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct SpawnFlags: u8 {
        const CLONE_RESOURCES = 1 << 0;
        const CLONE_CWD = 1 << 1;
    }
}

// used by tests...
#[allow(unused)]
pub fn function_spawn(
    name: Name,
    function: fn() -> !,
    argv: &[&str],
    flags: SpawnFlags,
) -> Result<usize, MapToError> {
    let mut this = this_state_mut();
    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        this.cwd()
    } else {
        make_path!("ram", "")
    };
    let cwd = Box::new(cwd.into_owned().unwrap());

    let mut page_table = PhysPageTable::create()?;
    let context =
        unsafe { CPUStatus::create(&mut page_table, argv, function as usize, false).unwrap() };
    let task = Task::new(name, 0, 0, cwd, page_table, context, 0);

    if flags.contains(SpawnFlags::CLONE_RESOURCES) {
        let mut state = task.state_mut().unwrap();

        let TaskState::Alive { resources, .. } = &mut *state else {
            unreachable!()
        };

        let clone = this.clone_resources();
        resources.overwrite_resources(clone);
    }

    let pid = super::add(task);
    Ok(pid)
}

pub fn spawn<T: Readable>(
    name: Name,
    reader: &T,
    argv: &[&str],
    flags: SpawnFlags,
) -> Result<usize, ElfError> {
    let this = this_state();
    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        this.cwd()
    } else {
        make_path!("ram", "")
    };

    let elf = Elf::new(reader)?;

    let current = super::current();
    let current_pid = current.pid;

    let task = Task::from_elf(name, 0, current_pid, cwd, elf, argv)?;

    if flags.contains(SpawnFlags::CLONE_RESOURCES) {
        let mut state = task.state_mut().unwrap();

        let TaskState::Alive { resources, .. } = &mut *state else {
            unreachable!()
        };

        drop(this);
        let mut this = this_state_mut();
        let clone = this.clone_resources();
        resources.overwrite_resources(clone);
    }

    let pid = super::add(task);
    Ok(pid)
}

/// spawns an elf process from a path
pub fn pspawn(name: Name, path: Path, argv: &[&str], flags: SpawnFlags) -> Result<usize, FSError> {
    let file = File::open(path)?;

    if file.kind() != InodeType::File {
        return Err(FSError::NotAFile);
    }

    spawn(name, &file, argv, flags).map_err(|_| FSError::NotExecuteable)
}

/// also ensures the cwd ends with /
/// will only Err if new_dir doesn't exists or is not a directory
#[no_mangle]
pub fn chdir(new_dir: Path) -> FSResult<()> {
    VFS_STRUCT.read().verify_path_dir(new_dir)?;

    let mut state = this_state_mut();
    let cwd = state.cwd_mut();

    if new_dir.is_absolute() {
        *cwd = new_dir.into_owned()?;
    } else {
        cwd.append(new_dir)?;
    }

    Ok(())
}

fn can_terminate(mut process_ppid: usize, process_pid: usize, terminator_pid: usize) -> bool {
    if process_ppid == terminator_pid || process_pid == terminator_pid {
        return true;
    }

    while process_ppid != 0 {
        if process_ppid == terminator_pid {
            return true;
        }

        let pprocess = super::find(|p| p.pid == process_ppid);
        process_ppid = pprocess
            .map(|process| process.ppid.load(Ordering::Relaxed))
            .unwrap_or(0);
    }

    false
}

fn terminate(process_pid: Pid, terminator_pid: Pid) {
    super::for_each(|process| {
        if process.pid == process_pid {
            process.kill(1, Some(terminator_pid));
        }
    });

    // moves the parentership of all processes with `ppid` as `process_pid` to `terminator_pid`
    // prevents orphan processes from being left behind
    // TODO: figure out if orphan processes should be killed
    super::for_each(|p| {
        _ = p.ppid.compare_exchange(
            process_pid,
            terminator_pid,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    });
}

#[no_mangle]
/// can only Err if pid doesn't belong to process
pub fn pkill(pid: Pid) -> Result<(), ()> {
    let current = super::current();
    let current_pid = current.pid;

    let (process_ppid, process_pid) = super::find(|p| p.pid == pid)
        .map(|process| (process.ppid.load(Ordering::Relaxed), process.pid))
        .ok_or(())?;

    if can_terminate(process_ppid, process_pid, current_pid) {
        terminate(process_pid, current_pid);
        return Ok(());
    }
    Err(())
}

#[no_mangle]
/// extends program break by `amount`
/// returns the new program break ptr
/// on fail returns null
pub fn sbrk(amount: isize) -> Result<*mut u8, ErrorStatus> {
    let current = super::current();
    let mut state = current.state_mut().unwrap();
    state.extend_data_by(amount).ok_or(ErrorStatus::OutOfMemory)
}
