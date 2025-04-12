use safa_utils::abi::raw::{RawSlice, RawSliceMut};
use safa_utils::abi::{self, raw};
use safa_utils::errors::SysResult;

use crate::drivers::vfs::expose::FileAttr;
use crate::threading::task::TaskMetadata;
use crate::utils::syscalls::{SyscallFFI, SyscallTable};
use crate::{
    arch::power,
    debug,
    drivers::vfs::{
        expose::{DirEntry, DirIter, DirIterRef, File, FileRef},
        CtlArgs,
    },
    threading::expose::SpawnFlags,
    utils::{errors::ErrorStatus, path::Path},
    VirtAddr,
};
use crate::{threading, time};

impl SyscallFFI for FileRef {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        FileRef::get(args).ok_or(ErrorStatus::InvaildResource)
    }
}

impl SyscallFFI for File {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        File::from_fd(args).ok_or(ErrorStatus::InvaildResource)
    }
}

impl SyscallFFI for DirIterRef {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        DirIterRef::get(args).ok_or(ErrorStatus::InvaildResource)
    }
}

impl SyscallFFI for DirIter {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        DirIter::from_ri(args).ok_or(ErrorStatus::InvaildResource)
    }
}
mod io;
mod processes;
mod utils;

#[inline(always)]
/// takes the number of the syscall and the arguments and returns an error as a u16 if it fails
/// this function is the final non-arch-specific layer between the kernel and the syscalls
/// it maps from arguments to syscall arguments
/// the way arguments are mapped is defined by the [`SyscallFFI`] trait
pub fn syscall(number: u16, a: usize, b: usize, c: usize, d: usize, e: usize) -> SysResult {
    #[inline(always)]
    fn inner(
        number: u16,
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        e: usize,
    ) -> Result<(), ErrorStatus> {
        let syscall = SyscallTable::try_from(number).map_err(|_| ErrorStatus::InvaildSyscall)?;
        match syscall {
            // utils
            SyscallTable::SysExit => utils::sysexit(a),
            SyscallTable::SysYield => Ok(utils::sysyield()),
            SyscallTable::SysSbrk => {
                utils::syssbrk(a as isize, SyscallFFI::make(b as *mut VirtAddr)?)
            }
            SyscallTable::SysGetCWD => {
                let path = <&mut [u8]>::make((a as *mut u8, b))?;
                let dest_len = Option::make(c as *mut usize)?;
                utils::sysgetcwd(path, dest_len)
            }
            SyscallTable::SysCHDir => {
                let path = <Path>::make((a as *const u8, b))?;
                utils::syschdir(path)
            }
            // io
            SyscallTable::SysGetDirEntry => {
                let path = <Path>::make((a as *const u8, b))?;
                let direntry = <&mut DirEntry>::make(c as *mut DirEntry)?;
                *direntry = DirEntry::get_from_path(path)?;
                Ok(())
            }
            SyscallTable::SysOpen => {
                let path = <Path>::make((a as *const u8, b))?;
                let dest_fd = Option::make(c as *mut usize)?;
                io::sysopen(path, dest_fd)
            }
            SyscallTable::SysDirIterOpen => {
                let dir_rd = FileRef::make(a)?;
                let dest_diriter = Option::make(b as *mut usize)?;
                io::sysdiriter_open(dir_rd, dest_diriter)
            }
            // TODO: SysClose and SysDirIterClose should be the same syscall
            SyscallTable::SysClose => Ok(drop(File::make(a)?)),
            SyscallTable::SysDirIterClose => Ok(drop(DirIter::make(a)?)),
            SyscallTable::SysDirIterNext => {
                let diriter_rd = DirIterRef::make(a)?;
                let direntry = <&mut DirEntry>::make(b as *mut DirEntry)?;
                io::sysdiriter_next(diriter_rd, direntry)
            }
            SyscallTable::SysCreate => {
                let path = <Path>::make((a as *const u8, b))?;
                io::syscreate(path)
            }
            SyscallTable::SysCreateDir => {
                let path = <Path>::make((a as *const u8, b))?;
                io::syscreatedir(path)
            }
            SyscallTable::SysWrite => {
                let fd = FileRef::make(a)?;
                let offset = b as isize;
                let buf = SyscallFFI::make((c as *const u8, d))?;
                let dest_wrote = Option::make(e as *mut usize)?;
                io::syswrite(fd, offset, buf, dest_wrote)
            }
            SyscallTable::SysRead => {
                let fd = FileRef::make(a)?;
                let offset = b as isize;
                let buf = SyscallFFI::make((c as *mut u8, d))?;
                let dest_read = Option::make(e as *mut usize)?;
                io::sysread(fd, offset, buf, dest_read)
            }
            SyscallTable::SysTruncate => {
                let fd = FileRef::make(a)?;
                let len = b as usize;
                io::systruncate(fd, len)
            }
            SyscallTable::SysSync => {
                let fd = FileRef::make(a)?;
                io::syssync(fd)
            }
            SyscallTable::SysFSize => {
                let fd = FileRef::make(a)?;
                let dest_size: Option<&mut usize> = Option::make(b as *mut usize)?;
                let file_size = fd.size();

                if let Some(size) = dest_size {
                    *size = file_size;
                }
                Ok(())
            }
            SyscallTable::SysFAttrs => {
                let fd = FileRef::make(a)?;
                let dest_attrs: Option<&mut FileAttr> = Option::make(b as *mut FileAttr)?;

                if let Some(attrs) = dest_attrs {
                    *attrs = fd.attrs();
                }
                Ok(())
            }
            SyscallTable::SysDup => {
                let fd = FileRef::make(a)?;
                let dest_fd = <&mut FileRef>::make(b as *mut FileRef)?;
                let fd = fd.dup();
                *dest_fd = fd;
                Ok(())
            }
            // processes
            SyscallTable::SysPSpawn => {
                #[inline(always)]
                fn into_bytes_slice<'a>(
                    args_raw: &RawSliceMut<RawSlice<u8>>,
                ) -> Result<&'a [&'a [u8]], ErrorStatus> {
                    if args_raw.len() == 0 {
                        return Ok(&[]);
                    }

                    let raw_slice: &mut [RawSlice<u8>] =
                        SyscallFFI::make((args_raw.as_mut_ptr(), args_raw.len()))?;
                    // unsafely creates a muttable reference to `raw_slice`
                    let double_slice: &mut [&[u8]] =
                        unsafe { &mut *(raw_slice as *const _ as *mut [&[u8]]) };

                    // maps every parent_slice[i] to &str
                    for (i, item) in raw_slice.iter().enumerate() {
                        double_slice[i] = <&[u8]>::make((item.as_ptr(), item.len()))?;
                    }

                    Ok(double_slice)
                }

                #[inline(always)]
                /// converts slice of raw pointers to a slice of strs which is used by pspawn as
                /// process arguments
                fn into_args_slice<'a>(
                    args_raw: &RawSliceMut<RawSlice<u8>>,
                ) -> Result<&'a [&'a str], ErrorStatus> {
                    if args_raw.len() == 0 {
                        return Ok(&[]);
                    }

                    let raw_slice: &mut [RawSlice<u8>] =
                        SyscallFFI::make((args_raw.as_mut_ptr(), args_raw.len()))?;
                    // unsafely creates a muttable reference to `raw_slice`
                    let double_slice: &mut [&str] =
                        unsafe { &mut *(raw_slice as *const _ as *mut [&str]) };

                    // maps every parent_slice[i] to &str
                    for (i, item) in raw_slice.iter().enumerate() {
                        double_slice[i] = <&str>::make((item.as_ptr(), item.len()))?;
                    }

                    Ok(double_slice)
                }

                fn as_rust(
                    this: &raw::processes::SpawnConfig,
                ) -> Result<
                    (
                        Option<&str>,
                        &[&str],
                        &[&[u8]],
                        SpawnFlags,
                        Option<TaskMetadata>,
                    ),
                    ErrorStatus,
                > {
                    let name = Option::<&str>::make((this.name.as_ptr(), this.name.len()))?;
                    let argv = into_args_slice(&this.argv)?;
                    let env = into_bytes_slice(&this.env)?;

                    let metadata: Option<&abi::raw::processes::TaskMetadata> = if this.version >= 1
                    {
                        Option::make(this.metadata)?
                    } else {
                        None
                    };

                    Ok((
                        name,
                        argv,
                        env,
                        this.flags.into(),
                        metadata.copied().map(Into::into),
                    ))
                }

                let path = <Path>::make((a as *const u8, b))?;

                let config = SyscallFFI::make(c as *const raw::processes::SpawnConfig)?;
                let (name, argv, env, flags, metadata) = as_rust(config)?;

                let dest_pid = Option::make(d as *mut usize)?;

                processes::syspspawn(name, path, argv, env, flags, metadata, dest_pid)
            }
            SyscallTable::SysMetaTake => {
                let dest_metadata = <&mut abi::raw::processes::TaskMetadata>::make(
                    a as *mut abi::raw::processes::TaskMetadata,
                )?;

                *dest_metadata = threading::expose::metadata_take()
                    .ok_or(ErrorStatus::Generic)?
                    .into();
                Ok(())
            }
            SyscallTable::SysCtl => {
                let fd = FileRef::make(a)?;
                let cmd = b as u16;
                let args = <&[usize]>::make((c as *const usize, d))?;
                let args = CtlArgs::new(args);
                Ok(fd.ctl(cmd, args)?)
            }
            SyscallTable::SysWait => {
                let dest_code = Option::make(b as *mut usize)?;
                processes::syswait(a, dest_code)
            }
            // power
            SyscallTable::SysShutdown => Ok(power::shutdown()),
            SyscallTable::SysReboot => Ok(power::reboot()),
            SyscallTable::SysUptime => Ok({
                let dest_uptime = <&mut u64>::make(a as *mut u64)?;
                *dest_uptime = time!();
            }),
            #[allow(unreachable_patterns)]
            syscall => {
                debug!(
                    SyscallTable,
                    "defined but unimplemented syscall {}({:?}) called with arguments {} {} {} {}",
                    number,
                    syscall,
                    a,
                    b,
                    c,
                    d
                );
                Err(ErrorStatus::InvaildSyscall)
            }
        }
    }

    // maps the results to an ErrorStatus
    let value = inner(number, a, b, c, d, e).into();
    value
}
