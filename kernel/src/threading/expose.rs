use core::arch::asm;

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};
use bitflags::bitflags;

use crate::{
    drivers::vfs::{
        expose::{close, fstat, open, read, DirEntry},
        FSError, FSResult, InodeType, VFS_STRUCT,
    },
    khalt,
    utils::elf::{Elf, ElfError},
};

use super::{
    resources,
    task::{Task, TaskInfo, TaskState},
    Pid,
};

#[no_mangle]
pub fn thread_exit(code: usize) {
    super::with_current(|current| current.kill(code, None));
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
        let found = super::find(
            |process| process.pid == pid,
            |process| process.state().exit_code(),
        );

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
    super::find(|p| p.pid == pid, |p| TaskInfo::from(&*p))
}

pub fn getpids() -> Vec<Pid> {
    let mut pids = Vec::with_capacity(super::pcount());
    super::for_each(|process| pids.push(process.pid));

    pids
}
bitflags! {
    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct SpawnFlags: u8 {
        const CLONE_RESOURCES = 1 << 0;
        const CLONE_CWD = 1 << 1;
    }
}

pub fn spawn(
    name: &str,
    elf_bytes: &[u8],
    argv: &[&str],
    flags: SpawnFlags,
) -> Result<usize, ElfError> {
    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        getcwd()
    } else {
        String::from("ram:/")
    };

    let elf = Elf::new(elf_bytes)?;

    let current_pid = super::with_current(|p| p.pid);

    let task = Task::from_elf(name.to_string(), 0, current_pid, cwd, elf, argv)?;

    if flags.contains(SpawnFlags::CLONE_RESOURCES) {
        let mut state = task.state_mut();

        let TaskState::Alive { resources, .. } = &mut *state else {
            unreachable!()
        };

        let clone = resources::clone_resources();
        resources.overwrite_resources(clone);
    }

    let pid = super::add_task(task);
    Ok(pid)
}

/// spawns an elf process from a path
pub fn pspawn(name: &str, path: &str, argv: &[&str], flags: SpawnFlags) -> Result<usize, FSError> {
    let file = open(path)?;

    let mut stat = unsafe { DirEntry::zeroed() };
    fstat(file, &mut stat)?;

    if stat.kind != InodeType::File {
        return Err(FSError::NotAFile);
    }

    let mut buffer = vec![0; stat.size];

    read(file, &mut buffer)?;
    close(file)?;
    spawn(name, &buffer, argv, flags).map_err(|_| FSError::NotExecuteable)
}

/// also ensures the cwd ends with /
/// will only Err if new_dir doesn't exists or is not a directory
#[no_mangle]
pub fn chdir(new_dir: &str) -> FSResult<()> {
    let new_dir = VFS_STRUCT.read().verify_path_dir(new_dir)?;

    super::with_current(move |current| {
        let mut state = current.state_mut();
        let cwd = state.cwd_mut();

        *cwd = new_dir;
        // TODO: implement a Path type with abillity to append paths to prevent this, and also to
        // prevent path's like ram:/dir/../dir/ from existing idiots
        if !cwd.ends_with('/') {
            cwd.push('/');
        }
        Ok(())
    })
}

#[no_mangle]
pub fn getcwd() -> String {
    super::with_current(|current| current.state().cwd().to_string())
}

fn can_terminate(mut process_ppid: usize, process_pid: usize, terminator_pid: usize) -> bool {
    if process_ppid == terminator_pid || process_pid == terminator_pid {
        return true;
    }

    while process_ppid != 0 {
        if process_ppid == terminator_pid {
            return true;
        }
        process_ppid = super::find(|p| p.pid == process_ppid, |process| process.ppid).unwrap_or(0);
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
        if p.ppid == process_pid {
            p.ppid = terminator_pid;
        }
    });
}

#[no_mangle]
/// can only Err if pid doesn't belong to process
pub fn pkill(pid: Pid) -> Result<(), ()> {
    let current_pid = super::with_current(|current| current.pid);
    if pid < current_pid {
        return Err(());
    }

    let (process_ppid, process_pid) =
        super::find(|p| p.pid == pid, |process| (process.ppid, process.pid)).ok_or(())?;
    if can_terminate(process_ppid, process_pid, current_pid) {
        terminate(process_pid, current_pid);
        return Ok(());
    }
    Err(())
}

#[no_mangle]
/// collects as much processes as it can in `buffer`
/// collects `buffer.len()` processes
/// if it didn't finish returns Err(())
pub fn pcollect(info: &mut [TaskInfo]) -> Result<(), ()> {
    let mut i = 0;

    super::while_each(|process| {
        if i >= info.len() {
            return false;
        }

        info[i] = TaskInfo::from(&*process);
        i += 1;
        true
    });

    Ok(())
}

#[no_mangle]
/// extends program break by `amount`
/// returns the new program break ptr
/// on fail returns null
pub fn sbrk(amount: isize) -> *mut u8 {
    super::with_current(|current| current.state_mut().extend_data_by(amount))
        .unwrap_or(core::ptr::null_mut())
}
