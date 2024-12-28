use core::arch::asm;

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};
use bitflags::bitflags;

use crate::{
    drivers::vfs::{
        expose::{fstat, open, read, DirEntry},
        FSError, FSResult, InodeType, VFS_STRUCT,
    },
    khalt,
    threading::processes::Process,
    utils::elf::{Elf, ElfError},
};

use super::processes::{ProcessInfo, ProcessState};

#[no_mangle]
pub fn thread_exit(code: usize) {
    super::with_current(|process| process.terminate(code, 0));
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
            |process| {
                if let ProcessState::Zombie(ref state) = process.state {
                    let exit_code = state.exit_code;
                    Some(exit_code)
                } else {
                    None
                }
            },
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
pub fn getinfo(pid: usize) -> Option<ProcessInfo> {
    super::find(|p| p.pid == pid, |p| p.info())
}

pub fn getpids() -> Vec<usize> {
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
        getcwd().to_string()
    } else {
        String::from("ram:/")
    };

    let elf = Elf::new(elf_bytes)?;

    let current_pid = super::with_current(|p| p.pid);
    let mut process = Process::from_elf(current_pid, elf, name, cwd, argv)?;

    let ProcessState::Alive(ref mut state) = process.state else {
        unreachable!()
    };
    // handles the flags
    if flags.contains(SpawnFlags::CLONE_RESOURCES) {
        let clone =
            super::with_current_state(|state| state.resource_manager.lock().clone_resources());
        state.resource_manager.lock().overwrite_resources(clone);
    }

    let pid = super::add_process(process);
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
    spawn(name, &buffer, argv, flags).map_err(|_| FSError::NotExecuteable)
}

/// also ensures the cwd ends with /
/// will only Err if new_dir doesn't exists or is not a directory
#[no_mangle]
pub fn chdir(new_dir: &str) -> FSResult<()> {
    let new_dir = VFS_STRUCT.read().verify_path_dir(new_dir)?;

    super::with_current_state(move |state| {
        state.current_dir = new_dir;
        // TODO: implement a Path type with abillity to append paths to prevent this, and also to
        // prevent path's like ram:/dir/../dir/ from existing idiots
        if !state.current_dir.ends_with('/') {
            state.current_dir.push('/');
        }
        Ok(())
    })
}

#[no_mangle]
pub fn getcwd() -> String {
    super::with_current_state(|state| state.current_dir.clone())
}

/// attempts to terminate `process` as `terminator_pid` and returns `Some(Ok())` if it was able to terminate it
/// returns `Err(terminator_parent_ppid)` if it wasn't able to terminate it
/// returns `Err(0)` if `process_ppod` doesn't belong to a process
/// it takes `process_ppid` as a process's parent pid (grandparent or even great-grandparent) and
/// terminates the process if that parent is the terminator
fn try_terminate(
    process: &mut Process,
    process_ppid: usize,
    terminator_pid: usize,
) -> Result<(), usize> {
    super::find(
        |p| p.pid == process_ppid,
        |process_parent| {
            if process_parent.pid == terminator_pid {
                process.terminate(1, terminator_pid);
                Ok(())
            } else {
                Err(process_parent.ppid)
            }
        },
    )
    .unwrap_or(Err(0))
}
#[no_mangle]
/// can only Err if pid doesn't belong to process
pub fn pkill(pid: usize) -> Result<(), ()> {
    super::with_current(|current| {
        let current_pid = current.pid;
        if pid < current_pid {
            return Err(());
        }

        super::find(
            |p| p.pid == pid,
            |process| {
                if process.ppid == current_pid || process.pid == current_pid {
                    process.terminate(1, current_pid);
                    return Ok(());
                }

                // loops through parents and checks if one of the great-grandparents is the current process
                let mut process_ppid = process.ppid;
                while process_ppid != 0 {
                    match try_terminate(process, process_ppid, current_pid) {
                        Err(ppid) => process_ppid = ppid,
                        Ok(()) => return Ok(()),
                    }
                }

                Err(())
            },
        )
        .ok_or(())?
    })
}

#[no_mangle]
/// collects as much processes as it can in `buffer`
/// collects `buffer.len()` processes
/// if it didn't finish returns Err(())
pub fn pcollect(info: &mut [ProcessInfo]) -> Result<(), ()> {
    let mut i = 0;

    super::while_each(|process| {
        if i >= info.len() {
            return false;
        }

        info[i] = process.info();
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
    super::with_current_state(|state| state.extend_data_by(amount)).unwrap_or(core::ptr::null_mut())
}
