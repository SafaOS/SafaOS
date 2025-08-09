//! This module contains functions related to creating (spawn)ing new processes.
use core::num::NonZero;

use alloc::{boxed::Box, sync::Arc};
use bitflags::bitflags;
use safa_abi::{
    fs::FSObjectType,
    process::{ProcessStdio, RawPSpawnConfig},
};
use thiserror::Error;

use crate::{
    drivers::vfs::FSError,
    fs::File,
    memory::paging::MapToError,
    process::{self, Pid, Process, resources::Ri},
    scheduler,
    thread::{ArcThread, ContextPriority},
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

impl From<safa_abi::process::SpawnFlags> for SpawnFlags {
    fn from(value: safa_abi::process::SpawnFlags) -> Self {
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
    ) -> Result<(Arc<Process>, ArcThread), SpawnError>,
) -> Result<Pid, SpawnError> {
    let current_process = process::current();
    let current_pid = current_process.pid();

    let cwd = if flags.contains(SpawnFlags::CLONE_CWD) {
        current_process.cwd().clone()
    } else {
        Box::new(path::make_path!("ram", "").into_owned().unwrap())
    };

    let new_pid = scheduler::process_list::add_pid();
    let (new_process, root_thread) = create_process(name, current_pid, new_pid, cwd)?;

    // Provides resources for the new process
    {
        let mut new_process_resources = new_process.resources_mut();
        let mut this_resources = current_process.resources_mut();

        let clone = if flags.contains(SpawnFlags::CLONE_RESOURCES) {
            this_resources.clone()
        } else {
            // clone only necessary resources
            let mut resources = heapless::Vec::<Ri, 3>::new();
            if let Some(stdin) = stdio.stdin.into() {
                _ = resources.push(stdin);
            }

            if let Some(stdout) = stdio.stdout.into() {
                _ = resources.push(stdout);
            }

            if let Some(stderr) = stdio.stderr.into() {
                _ = resources.push(stderr);
            }

            this_resources
                .clone_specific_resources(&resources)
                .map_err(|()| FSError::InvalidResource)?
        };

        new_process_resources.overwrite_resources(clone);
    }

    scheduler::add_process(new_process, root_thread, None);
    Ok(new_pid)
}

fn spawn<T: Readable>(
    name: Name,
    reader: &T,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    priority: ContextPriority,
    stdio: ProcessStdio,
    custom_stack_size: Option<NonZero<usize>>,
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
    custom_stack_size: Option<NonZero<usize>>,
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

use crate::utils::ffi::ForeignTryAccept;
use safa_abi::errors::ErrorStatus;

pub struct PSpawnConfig<'a> {
    name: Option<&'a str>,
    args: Option<&'a [&'a str]>,
    envv: Option<&'a [&'a [u8]]>,
    stdio: Option<ProcessStdio>,

    priority: ContextPriority,
    custom_stack_size: Option<NonZero<usize>>,
    flags: SpawnFlags,
}

impl<'a> TryFrom<&'a RawPSpawnConfig> for PSpawnConfig<'a> {
    type Error = ErrorStatus;
    fn try_from(value: &'a RawPSpawnConfig) -> Result<Self, Self::Error> {
        let name: Option<&str> = value.name.try_accept()?;

        let args: Option<&[&str]> = value.argv.try_accept()?;
        let stdio: Option<&ProcessStdio> = value.stdio.try_accept()?;
        let stdio = stdio.map(|r| *r);

        let envv: Option<&[&[u8]]> = if value.revision >= 1 {
            value.env.try_accept()?
        } else {
            None
        };

        let priority: ContextPriority = if value.revision >= 2 {
            value.priority.into()
        } else {
            ContextPriority::Medium
        };

        let custom_stack_size: Option<NonZero<usize>> = if value.revision >= 3 {
            value.custom_stack_size.into()
        } else {
            None
        };

        let flags = value.flags.into();

        Ok(Self {
            name,
            args,
            envv,
            stdio,
            priority,
            custom_stack_size,
            flags,
        })
    }
}

impl<'a> PSpawnConfig<'a> {
    pub const fn name(&self) -> Option<&str> {
        self.name
    }

    pub const fn args(&self) -> &[&str] {
        match self.args {
            Some(args) => args,
            None => &[],
        }
    }

    pub const fn envv(&self) -> &[&[u8]] {
        match self.envv {
            Some(envv) => envv,
            None => &[],
        }
    }

    pub const fn stdio(&self) -> Option<&ProcessStdio> {
        self.stdio.as_ref()
    }

    pub const fn priority(&self) -> ContextPriority {
        self.priority
    }

    pub const fn custom_stack_size(&self) -> Option<NonZero<usize>> {
        self.custom_stack_size
    }

    pub const fn flags(&self) -> SpawnFlags {
        self.flags
    }
}
