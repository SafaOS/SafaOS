use safa_abi::process::{ContextPriority, ProcessStdio, RawPSpawnConfig, RawTSpawnConfig};

use crate::process::spawn::PSpawnConfig;
use crate::process::{self, Pid, spawn::SpawnFlags};
use crate::{VirtAddr, utils::types::Name};
use core::fmt::Write;

use crate::utils::path::Path;
use safa_abi::errors::ErrorStatus;

use super::ffi::SyscallFFI;
use crate::thread::{self, Tid};
use macros::syscall_handler;

#[syscall_handler]
fn syst_fut_wait(addr: &mut u32, val: u32, timeout_ms: u64, wait_results: Option<&mut bool>) {
    let results = unsafe { thread::current::wait_for_futex(addr, val, timeout_ms) };
    if let Some(wait_results) = wait_results {
        *wait_results = results;
    }
}

#[syscall_handler]
fn syst_fut_wake(addr: &mut u32, n: usize, wake_results: Option<&mut usize>) {
    let num_threads = process::current::wake_futex(addr, n);
    if let Some(wake_results) = wake_results {
        *wake_results = num_threads;
    }
}

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
    custom_stack_size: Option<usize>,
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
        priority.unwrap_or(ContextPriority::Medium),
        stdio,
        custom_stack_size,
    )?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}

struct TSpawnConfig {
    argument_ptr: VirtAddr,
    priority: Option<ContextPriority>,
    cpu: Option<u8>,
    custom_stack_size: Option<usize>,
}

impl TryFrom<&RawTSpawnConfig> for TSpawnConfig {
    type Error = ErrorStatus;
    fn try_from(value: &RawTSpawnConfig) -> Result<Self, Self::Error> {
        let argument_ptr = VirtAddr::from_ptr(value.argument_ptr);
        let priority = value.priority.into();
        let cpu = value.cpu.into();
        let custom_stack_size = if value.revision >= 1 {
            value.custom_stack_size.into()
        } else {
            None
        };

        Ok(TSpawnConfig {
            argument_ptr,
            priority,
            cpu,
            custom_stack_size,
        })
    }
}

#[syscall_handler]
fn sys_tspawn(
    entry_point: VirtAddr,
    raw_config: &RawTSpawnConfig,
    target_tid: Option<&mut Tid>,
) -> Result<(), ErrorStatus> {
    let config: TSpawnConfig = raw_config.try_into()?;

    let thread_tid = process::current::thread_spawn(
        entry_point,
        config.argument_ptr,
        config.priority,
        config.cpu.map(|v| v as usize /* too lazy to change */),
        config.custom_stack_size,
    )
    .map_err(|_| ErrorStatus::MMapError)?;
    if let Some(target_tid) = target_tid {
        *target_tid = thread_tid;
    }
    Ok(())
}
