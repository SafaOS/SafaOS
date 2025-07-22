//! This module contains functions related to creating (spawn)ing new processes.
use alloc::{boxed::Box, sync::Arc};
use bitflags::bitflags;
use safa_abi::raw::{
    self,
    io::FSObjectType,
    processes::{ContextPriority, ProcessStdio},
};
use thiserror::Error;

use crate::{
    drivers::vfs::FSError,
    fs::File,
    memory::paging::MapToError,
    process::{self, Pid, Process},
    scheduler,
    thread::Thread,
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::{self, Path, PathBuf},
        types::Name,
    },
};

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
    #[error("failed to map memory: {0}")]
    MapToError(#[from] MapToError),
    #[error("failed to parse elf to memory {0}")]
    ElfError(#[from] ElfError),
    #[error("fs error while creating process {0}")]
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
    let current_process = process::current();
    let current_pid = current_process.pid();

    let current_state = current_process.state();

    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        current_state.cwd()
    } else {
        path::make_path!("ram", "")
    };

    let new_pid = scheduler::add_pid();

    let cwd = Box::new(cwd.into_owned().unwrap());
    let (new_process, root_thread) = create_process(name, current_pid, new_pid, cwd)?;

    // Provides resources for the new process
    {
        let mut new_state = new_process.state_mut();

        let Some(new_process_resources) = new_state.resource_manager_mut() else {
            unreachable!();
        };

        drop(current_state);
        let mut this_state = current_process.state_mut();

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

            if !resources.is_empty() {
                this_state
                    .clone_specific_resources(&resources)
                    .map_err(|()| FSError::InvalidResource)?
            } else {
                alloc::vec::Vec::new()
            }
        };

        new_process_resources.overwrite_resources(clone);
    }

    let pid = scheduler::add_process(new_process, root_thread);
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
    let file = File::open_all(path)?;

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
