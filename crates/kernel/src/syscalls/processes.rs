use safa_utils::abi::{
    self,
    raw::{
        processes::{AbiStructures, TaskStdio},
        RawSlice, RawSliceMut,
    },
};

use crate::{threading::Pid, utils::types::Name};
use core::fmt::Write;

use crate::{
    threading::{self, expose::SpawnFlags},
    utils::{errors::ErrorStatus, path::Path},
};

use super::SyscallFFI;
use macros::syscall_handler;

#[syscall_handler]
fn syswait(pid: Pid, dest_code: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let code = threading::expose::wait(pid);
    if let Some(dest_code) = dest_code {
        *dest_code = code;
    }
    Ok(())
}

fn syspspawn_inner(
    name: Option<&str>,
    path: Path,
    argv: &[&str],
    env: &[&[u8]],
    flags: SpawnFlags,
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
    config: &abi::raw::processes::SpawnConfig,
    dest_pid: Option<&mut Pid>,
) -> Result<(), ErrorStatus> {
    fn as_rust(
        this: &abi::raw::processes::SpawnConfig,
    ) -> Result<
        (
            Option<&str>,
            &[&str],
            &[&[u8]],
            SpawnFlags,
            Option<TaskStdio>,
        ),
        ErrorStatus,
    > {
        let name = Option::<&str>::make((this.name.as_ptr(), this.name.len()))?;
        let argv = into_args_slice(&this.argv)?;
        let env = into_bytes_slice(&this.env)?;

        let stdio: Option<&abi::raw::processes::TaskStdio> = if this.version >= 1 {
            Option::make(this.stdio)?
        } else {
            None
        };

        Ok((name, argv, env, this.flags.into(), stdio.copied()))
    }

    let (name, argv, env, flags, stdio) = as_rust(config)?;

    let results = syspspawn_inner(name, path, argv, env, flags, stdio)?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}
