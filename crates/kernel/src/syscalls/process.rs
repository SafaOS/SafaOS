use crate::VirtAddr;
use crate::thread::ContextPriority;
use crate::utils::io::Cursor;
use safa_abi::process::{ProcessStdio, RawPSpawnConfig};

use crate::process::spawn::PSpawnConfig;
use crate::process::{self, Pid, spawn::SpawnFlags};
use crate::utils::types::Name;
use core::fmt::Write;
use core::num::NonZero;

use crate::utils::path::Path;
use safa_abi::errors::ErrorStatus;

use super::ffi::SyscallFFI;
use crate::thread::{self, Tid};
use macros::syscall_handler;

#[syscall_handler]
fn sysp_wait(pid: Pid, dest_code: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let code = thread::current::wait_for_process(pid).ok_or(ErrorStatus::InvalidPid)?;
    if let Some(dest_code) = dest_code {
        *dest_code = code;
    }
    Ok(())
}

#[syscall_handler]
fn syst_wait(tid: Tid) -> Result<(), ErrorStatus> {
    thread::current::wait_for_thread(tid).ok_or(ErrorStatus::InvalidTid)?;
    Ok(())
}

#[syscall_handler]
fn sysp_try_cleanup(pid: Pid, dest_exit_code: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let cleaned_up = process::current::try_cleanup_process(pid)?;
    if let Some(exit_code) = cleaned_up {
        if let Some(dest_exit_code) = dest_exit_code {
            *dest_exit_code = exit_code;
        }
        Ok(())
    } else {
        Err(ErrorStatus::Generic)
    }
}

fn syspspawn_inner(
    name: Option<&str>,
    path: Path,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
    priority: ContextPriority,
    stdio: Option<ProcessStdio>,
    custom_stack_size: Option<NonZero<usize>>,
) -> Result<Pid, ErrorStatus> {
    let name = match name {
        Some(raw) => Name::try_from(raw).map_err(|()| ErrorStatus::StrTooLong)?,
        None => {
            let mut name = Name::new();
            _ = name.write_fmt(format_args!("{path}"));
            name
        }
    };

    let results = process::spawn::pspawn(
        name,
        path,
        argv,
        env,
        flags,
        priority,
        stdio.unwrap_or_default(),
        custom_stack_size,
    )?;
    Ok(results)
}

#[syscall_handler]
fn syspspawn(
    path: Path,
    raw_config: &RawPSpawnConfig,
    dest_pid: Option<&mut Pid>,
) -> Result<(), ErrorStatus> {
    let config: PSpawnConfig = raw_config.try_into()?;

    let name = config.name();
    let argv = config.args();
    let env = config.envv();
    let flags = config.flags();
    let priority = config.priority();
    let stdio = config.stdio().map(|p| *p);
    let custom_stack_size = config.custom_stack_size();

    let results = syspspawn_inner(
        name,
        path,
        argv,
        env,
        flags,
        priority,
        stdio,
        custom_stack_size,
    )?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}

#[syscall_handler]
fn sysp_sbrk(amount: isize, results: &mut VirtAddr) -> Result<(), ErrorStatus> {
    let res = process::current::extend_data_break(amount)?;
    *results = VirtAddr::from_ptr(res);
    Ok(())
}

#[syscall_handler]
fn syschdir(path: Path) -> Result<(), ErrorStatus> {
    process::current::chdir(path).map_err(|err| err.into())
}

#[syscall_handler]
/// gets the current working directory in `path` and puts the length of the gotten path in
/// `dest_len` if it is not null
/// returns ErrorStatus::Generic if the path is too long to fit in the given buffer `path`
fn sysgetcwd(path: &mut [u8], dest_len: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let this_process = process::current();

    let cwd = this_process.cwd();
    let cwd = cwd.as_path();

    let len = cwd.len();
    if let Some(dest_len) = dest_len {
        *dest_len = len;
    }

    let mut cursor = Cursor::new(path);
    write!(&mut cursor, "{cwd}").map_err(|_| ErrorStatus::Generic)?;
    Ok(())
}
