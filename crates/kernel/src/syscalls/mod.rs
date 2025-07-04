use safa_utils::errors::SysResult;

use crate::drivers::vfs::expose::FileAttr;
use crate::threading::{resources, Pid};
use crate::time;
use crate::utils::syscalls::{SyscallFFI, SyscallTable};
use crate::{
    arch::power,
    drivers::vfs::expose::{DirEntry, DirIter, File, FileRef},
    utils::errors::ErrorStatus,
    VirtAddr,
};

impl SyscallFFI for File {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        File::from_fd(args).ok_or(ErrorStatus::InvalidResource)
    }
}

impl SyscallFFI for DirIter {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        DirIter::from_ri(args).ok_or(ErrorStatus::InvalidResource)
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
        let syscall = SyscallTable::try_from(number).map_err(|_| ErrorStatus::InvalidSyscall)?;
        match syscall {
            // utils
            SyscallTable::SysExit => utils::sysexit(a),
            SyscallTable::SysYield => Ok(utils::sysyield()),
            SyscallTable::SysSbrk => utils::syssbrk_raw(a, b as *mut VirtAddr),
            SyscallTable::SysGetCWD => utils::sysgetcwd_raw((a as *mut u8, b), c as *mut usize),
            SyscallTable::SysCHDir => utils::syschdir_raw((a as *const u8, b)),
            // io
            SyscallTable::SysGetDirEntry => {
                io::sysget_direntry_raw((a as *const u8, b), c as *mut DirEntry)
            }
            SyscallTable::SysOpenAll => io::sysopen_all_raw((a as *const u8, b), c as *mut usize),
            SyscallTable::SysOpen => io::sysopen_raw((a as *const u8, b), c, d as *mut usize),
            SyscallTable::SysRemovePath => io::sysremove_path_raw((a as *const u8, b)),
            SyscallTable::SysDirIterOpen => io::sysdiriter_open_raw(a, b as *mut usize),
            SyscallTable::SysDestroyResource => {
                resources::remove_resource(a).ok_or(ErrorStatus::InvalidResource)
            }
            SyscallTable::SysDirIterClose => Ok(drop(DirIter::make(a)?)),
            SyscallTable::SysDirIterNext => io::sysdiriter_next_raw(a, b as *mut DirEntry),
            SyscallTable::SysCreate => io::syscreate_raw((a as *const u8, b)),
            SyscallTable::SysCreateDir => io::syscreatedir_raw((a as *const u8, b)),
            SyscallTable::SysWrite => io::syswrite_raw(a, b, (c as *const u8, d), e as *mut usize),
            SyscallTable::SysRead => io::sysread_raw(a, b, (c as *mut u8, d), e as *mut usize),
            SyscallTable::SysTruncate => io::systruncate_raw(a, b),
            SyscallTable::SysSync => io::syssync_raw(a),
            SyscallTable::SysFSize => io::sysfsize_raw(a, b as *mut usize),
            SyscallTable::SysFAttrs => io::sysattrs_raw(a, b as *mut FileAttr),
            SyscallTable::SysDup => io::sysdup_raw(a, b as *mut FileRef),
            SyscallTable::SysCtl => io::sysctl_raw(a, b, (c as *const usize, d)),
            // processes
            SyscallTable::SysPSpawn => {
                processes::syspspawn_raw((a as *const u8, b), c as *const _, d as *mut Pid)
            }
            SyscallTable::SysWait => processes::syswait_raw(a, b as *mut usize),
            // power
            SyscallTable::SysShutdown => power::shutdown(),
            SyscallTable::SysReboot => power::reboot(),
            SyscallTable::SysUptime => Ok({
                let dest_uptime = <&mut u64>::make(a as *mut u64)?;
                *dest_uptime = time!(ms);
            }),
        }
    }

    // maps the results to an ErrorStatus
    let value = inner(number, a, b, c, d, e).into();
    value
}
