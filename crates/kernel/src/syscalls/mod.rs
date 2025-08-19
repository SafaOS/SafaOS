use core::sync::atomic::AtomicU32;

use safa_abi::errors::{ErrorStatus, SysResult};
use safa_abi::fs::{DirEntry, FileAttr};
use safa_abi::syscalls::SyscallTable;

use crate::fs::DirIter;
use crate::process::Pid;
use crate::process::resources::{self, Ri};
use crate::syscalls::ffi::SyscallFFI;
use crate::thread::Tid;

use crate::time;
use crate::{VirtAddr, arch::power};

pub mod ffi;
mod fs;
mod io;
/// SysMem syscalls implementation
mod mem;
/// SysP syscalls implementation
mod process;
/// SysSock syscalls implementation
mod sockets;
/// SysT syscalls implementation
mod thread;

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
            // IO related syscalls
            SyscallTable::SysFDirIterOpen => io::sysdiriter_open_raw(a, b as *mut usize),
            SyscallTable::SysDirIterClose => Ok(drop(DirIter::make(a)?)),
            SyscallTable::SysDirIterNext => io::sysdiriter_next_raw(a, b as *mut DirEntry),
            SyscallTable::SysIOWrite => {
                io::syswrite_raw(a, b, (c as *const u8, d), e as *mut usize)
            }
            SyscallTable::SysIORead => io::sysread_raw(a, b, (c as *mut u8, d), e as *mut usize),
            SyscallTable::SysIOTruncate => io::systruncate_raw(a, b),
            SyscallTable::SysIOSync => io::syssync_raw(a),
            SyscallTable::SysFSize => io::sysfsize_raw(a, b as *mut usize),
            SyscallTable::SysFAttrs => io::sysattrs_raw(a, b as *mut FileAttr),
            SyscallTable::SysIOCommand => io::sysio_command_raw(a, b, c),
            // Resources related syscalls
            SyscallTable::SysRDestroy => {
                if !resources::remove_resource(a) {
                    return Err(ErrorStatus::UnknownResource);
                }

                Ok(())
            }
            SyscallTable::SysRDup => io::sysdup_raw(a, b as *mut Ri),
            // FS related operations
            SyscallTable::SysFGetDirEntry => {
                fs::sysget_direntry_raw((a as *const u8, b), c as *mut DirEntry)
            }
            SyscallTable::SysFSOpenAll => fs::sysopen_all_raw((a as *const u8, b), c as *mut usize),
            SyscallTable::SysFSOpen => fs::sysopen_raw((a as *const u8, b), c, d as *mut usize),
            SyscallTable::SysFSRemovePath => fs::sysremove_path_raw((a as *const u8, b)),
            SyscallTable::SysFSCreate => fs::syscreate_raw((a as *const u8, b)),
            SyscallTable::SysFSCreateDir => fs::syscreatedir_raw((a as *const u8, b)),
            // processes
            SyscallTable::SysPSbrk => process::sysp_sbrk_raw(a, b as *mut VirtAddr),
            SyscallTable::SysPGetCWD => process::sysgetcwd_raw((a as *mut u8, b), c as *mut usize),
            SyscallTable::SysPCHDir => process::syschdir_raw((a as *const u8, b)),
            SyscallTable::SysPSpawn => {
                process::syspspawn_raw((a as *const u8, b), c as *const _, d as *mut Pid)
            }
            SyscallTable::SysTSpawn => thread::sys_tspawn_raw(a, b as *const _, c as *mut Tid),
            SyscallTable::SysPExit => crate::process::current::exit(a),
            SyscallTable::SysTExit => crate::thread::current::exit(a),
            SyscallTable::SysTYield => Ok(crate::thread::current::yield_now()),
            SyscallTable::SysTSleep => Ok(crate::thread::current::sleep_for_ms(a as u64)),
            SyscallTable::SysTFutWait => {
                thread::syst_fut_wait_raw(a as *const AtomicU32, b, c, d as *mut bool)
            }
            SyscallTable::SysTFutWake => {
                thread::syst_fut_wake_raw(a as *const AtomicU32, b, c as *mut usize)
            }
            SyscallTable::SysPTryCleanUp => process::sysp_try_cleanup_raw(a, b as *mut usize),
            SyscallTable::SysPWait => process::sysp_wait_raw(a, b as *mut usize),
            SyscallTable::SysTWait => process::syst_wait_raw(a),
            // power
            SyscallTable::SysShutdown => power::shutdown(),
            SyscallTable::SysReboot => power::reboot(),
            SyscallTable::SysUptime => Ok({
                let dest_uptime = <&mut u64>::make(a as *mut u64)?;
                *dest_uptime = time!(ms);
            }),
            // Memory
            SyscallTable::SysMemMap => {
                mem::sysmem_map_raw(a as *const _, b, c as *mut _, d as *mut _)
            }
            SyscallTable::SysMemShmCreate => mem::sysshm_create_raw(a, b, c as *mut _, d as *mut _),
            SyscallTable::SysMemShmOpen => mem::sysshm_open_raw(a, b, c as *mut _),
            // Sockets
            SyscallTable::SysSockCreate => sockets::syssock_create_raw(a, b, c, d as *mut _),
            SyscallTable::SysSockBind => sockets::syssock_bind_raw(a, b as *const _, c),
            SyscallTable::SysSockListen => sockets::syssock_listen_raw(a, b),
            SyscallTable::SysSockAccept => {
                sockets::syssock_accept_raw(a, b as *mut _, c as *mut _, d as *mut _)
            }
            SyscallTable::SysSockConnect => {
                sockets::syssock_connect_raw(a, b as *const _, c, d as *mut _)
            }
        }
    }

    // maps the results to an ErrorStatus
    let value = inner(number, a, b, c, d, e).into();
    value
}
