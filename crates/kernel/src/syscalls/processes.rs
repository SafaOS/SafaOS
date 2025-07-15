use safa_utils::abi::{
    self,
    raw::{
        RawSlice, RawSliceMut,
        processes::{AbiStructures, ContextPriority, PSpawnConfig, TSpawnConfig, TaskStdio},
    },
};

use crate::{
    VirtAddr,
    threading::{Pid, cpu_context::Cid},
    utils::types::Name,
};
use core::fmt::Write;

use crate::{
    threading::{self, expose::SpawnFlags},
    utils::{errors::ErrorStatus, path::Path},
};

use super::SyscallFFI;
use macros::syscall_handler;

#[syscall_handler]
fn sysp_wait(pid: Pid, dest_code: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let code = threading::expose::wait_for_task(pid).ok_or(ErrorStatus::InvalidPid)?;
    if let Some(dest_code) = dest_code {
        *dest_code = code;
    }
    Ok(())
}

#[syscall_handler]
fn syst_wait(tid: Cid) -> Result<(), ErrorStatus> {
    threading::expose::wait_for_thread(tid).ok_or(ErrorStatus::InvalidTid)?;
    Ok(())
}

#[syscall_handler]
fn sysp_try_cleanup(pid: Pid, dest_exit_code: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let cleaned_up =
        threading::expose::try_cleanup_process(pid).map_err(|()| ErrorStatus::InvalidPid)?;
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
    stdio: Option<TaskStdio>,
) -> Result<Pid, ErrorStatus> {
    let name = match name {
        Some(raw) => Name::try_from(raw).map_err(|()| ErrorStatus::StrTooLong)?,
        None => {
            let mut name = Name::new();
            _ = name.write_fmt(format_args!("{path}"));
            name
        }
    };

    let results = threading::expose::pspawn(
        name,
        path,
        argv,
        env,
        flags,
        priority,
        AbiStructures {
            stdio: stdio.unwrap_or_default(),
        },
    )?;
    Ok(results)
}

#[inline(always)]
fn into_bytes_slice<'a>(
    args_raw: &RawSliceMut<RawSlice<u8>>,
) -> Result<&'a [&'a [u8]], ErrorStatus> {
    if args_raw.len() == 0 {
        return Ok(&[]);
    }

    let raw_slice: &mut [RawSlice<u8>] = SyscallFFI::make((args_raw.as_mut_ptr(), args_raw.len()))?;
    // unsafely creates a muttable reference to `raw_slice`
    let double_slice: &mut [&[u8]] = unsafe { &mut *(raw_slice as *const _ as *mut [&[u8]]) };

    // maps every parent_slice[i] to &str
    for (i, item) in raw_slice.iter().enumerate() {
        double_slice[i] = <&[u8]>::make((item.as_ptr(), item.len()))?;
    }

    Ok(double_slice)
}

#[inline(always)]
/// converts slice of raw pointers to a slice of strs which is used by pspawn as
/// process arguments
fn into_args_slice<'a>(args_raw: &RawSliceMut<RawSlice<u8>>) -> Result<&'a [&'a str], ErrorStatus> {
    if args_raw.len() == 0 {
        return Ok(&[]);
    }

    let raw_slice: &mut [RawSlice<u8>] = SyscallFFI::make((args_raw.as_mut_ptr(), args_raw.len()))?;
    // unsafely creates a muttable reference to `raw_slice`
    let double_slice: &mut [&str] = unsafe { &mut *(raw_slice as *const _ as *mut [&str]) };

    // maps every parent_slice[i] to &str
    for (i, item) in raw_slice.iter().enumerate() {
        double_slice[i] = <&str>::make((item.as_ptr(), item.len()))?;
    }

    Ok(double_slice)
}

#[syscall_handler]
fn syspspawn(
    path: Path,
    config: &PSpawnConfig,
    dest_pid: Option<&mut Pid>,
) -> Result<(), ErrorStatus> {
    fn as_rust(
        this: &PSpawnConfig,
    ) -> Result<
        (
            Option<&str>,
            &[&str],
            &[&[u8]],
            SpawnFlags,
            Option<TaskStdio>,
            Option<ContextPriority>,
        ),
        ErrorStatus,
    > {
        let name = Option::<&str>::make((this.name.as_ptr(), this.name.len()))?;
        let argv = into_args_slice(&this.argv)?;
        let env = into_bytes_slice(&this.env)?;

        let stdio: Option<&abi::raw::processes::TaskStdio> = if this.revision >= 1 {
            Option::make(this.stdio)?
        } else {
            None
        };

        let priority: Option<ContextPriority> = if this.revision >= 2 {
            this.priority.into()
        } else {
            None
        };

        Ok((name, argv, env, this.flags.into(), stdio.copied(), priority))
    }

    let (name, argv, env, flags, stdio, priority) = as_rust(config)?;

    let results = syspspawn_inner(
        name,
        path,
        argv,
        env,
        flags,
        priority.unwrap_or(ContextPriority::Medium),
        stdio,
    )?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}

#[syscall_handler]
fn sys_tspawn(
    entry_point: VirtAddr,
    config: &TSpawnConfig,
    target_cid: Option<&mut Cid>,
) -> Result<(), ErrorStatus> {
    let (argument_ptr, priority, cpu) = config.into_rust();
    let argument_ptr = VirtAddr::from_ptr(argument_ptr);

    let thread_cid = threading::expose::thread_spawn(entry_point, argument_ptr, priority, cpu)
        .map_err(|_| ErrorStatus::MMapError)?;
    if let Some(target_cid) = target_cid {
        *target_cid = thread_cid;
    }
    Ok(())
}
