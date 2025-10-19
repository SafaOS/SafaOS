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
pub fn syscall(
    number: u16,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
) -> SysResult {
    #[inline(always)]
    fn inner(
        number: u16,
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        e: usize,
        f: usize,
    ) -> Result<usize, ErrorStatus> {
        let syscall = SyscallTable::try_from(number).map_err(|_| ErrorStatus::InvalidSyscall)?;
        match syscall {
            // IO related syscalls
            SyscallTable::SysFDirIterOpen => io::sysdiriter_open_raw(a as Ri),
            SyscallTable::SysDirIterClose => {
                drop(DirIter::make(a as Ri)?);
                Ok(0)
            }
            SyscallTable::SysDirIterNext => io::sysdiriter_next_raw(a as Ri, b as *mut DirEntry),
            SyscallTable::SysIOWrite => io::syswrite_raw(a as Ri, b as isize, (c as *const u8, d)),
            SyscallTable::SysIORead => io::sysread_raw(a as Ri, b as isize, (c as *mut u8, d)),
            SyscallTable::SysIOTruncate => io::systruncate_raw(a as Ri, b),
            SyscallTable::SysIOSync => io::syssync_raw(a as Ri),
            SyscallTable::SysIOPoll => io::sysio_poll_raw((a as *mut _, b), c as u64),
            SyscallTable::SysFSize => io::sysfsize_raw(a as Ri),
            SyscallTable::SysFAttrs => io::sysattrs_raw(a as Ri, b as *mut FileAttr),
            SyscallTable::SysIOCommand => io::sysio_command_raw(a as Ri, b as u16, c as u64),
            SyscallTable::SysVTTYAlloc => io::sysvtty_alloc_raw(a as *mut Ri, b as *mut Ri),
            // Resources related syscalls
            SyscallTable::SysRDestroy => {
                if !resources::remove_resource(a as Ri) {
                    return Err(ErrorStatus::UnknownResource);
                }

                Ok(0)
            }
            SyscallTable::SysRClone => io::sysclone_raw(a as Ri),
            // FS related operations
            SyscallTable::SysFGetDirEntry => {
                fs::sysget_direntry_raw((a as *const u8, b), c as *mut DirEntry)
            }
            SyscallTable::SysFSOpenAll => fs::sysopen_all_raw((a as *const u8, b)),
            SyscallTable::SysFSOpen => fs::sysopen_raw((a as *const u8, b), c as u8),
            SyscallTable::SysFSRemovePath => fs::sysremove_path_raw((a as *const u8, b)),
            SyscallTable::SysFSCreate => fs::syscreate_raw((a as *const u8, b)),
            SyscallTable::SysFSCreateDir => fs::syscreatedir_raw((a as *const u8, b)),
            // processes
            SyscallTable::SysPSbrk => process::sysp_sbrk_raw(a as isize, b as *mut VirtAddr),
            SyscallTable::SysPGetCWD => process::sysgetcwd_raw((a as *mut u8, b)),
            SyscallTable::SysPCHDir => process::syschdir_raw((a as *const u8, b)),
            SyscallTable::SysPSpawn => process::syspspawn_raw((a as *const u8, b), c as *const _),
            SyscallTable::SysTSpawn => thread::sys_tspawn_raw(a, b as *const _),
            SyscallTable::SysPExit => crate::process::current::exit(a),
            SyscallTable::SysTExit => crate::thread::current::exit(a),
            SyscallTable::SysTYield => {
                crate::thread::current::yield_now();
                Ok(0)
            }
            SyscallTable::SysTSleep => {
                crate::thread::current::sleep_for_ms(a as u64)?;
                Ok(0)
            }
            SyscallTable::SysTFutWait => {
                thread::syst_fut_wait_raw(a as *const AtomicU32, b as u32, c as u64)
            }
            SyscallTable::SysTFutWake => thread::syst_fut_wake_raw(a as *const AtomicU32, b),
            SyscallTable::SysPTryCleanUp => {
                process::sysp_try_cleanup_raw(a as Pid, b as *mut usize)
            }
            SyscallTable::SysPWait => process::sysp_wait_raw(a as Pid, b as *mut usize),
            SyscallTable::SysTWait => process::syst_wait_raw(a as Tid),
            // power
            SyscallTable::SysShutdown => power::shutdown(),
            SyscallTable::SysReboot => power::reboot(),
            SyscallTable::SysUptime => {
                let dest_uptime = <&mut u64>::make(a as *mut u64)?;
                *dest_uptime = time!(ms);

                Ok(0)
            }
            // Memory
            SyscallTable::SysMemMap => mem::sysmem_map_raw(a as *const _, b, c as *mut _),
            SyscallTable::SysMemShmCreate => mem::sysshm_create_raw(a, b, c as *mut _),
            SyscallTable::SysMemShmOpen => mem::sysshm_open_raw(a, b),
            // Sockets
            SyscallTable::SysSockCreate => sockets::syssock_create_raw(a, b, c as u32),
            SyscallTable::SysSockBind => sockets::syssock_bind_raw(a as Ri, (b as *const _, c)),
            SyscallTable::SysSockListen => sockets::syssock_listen_raw(a as Ri, b),
            SyscallTable::SysSockAccept => sockets::syssock_accept_raw(a as Ri, b as *mut _),
            SyscallTable::SysSockConnect => {
                sockets::syssock_connect_raw(a as Ri, (b as *const _, c))
            }
            SyscallTable::SysSockSendTo => {
                sockets::syssock_sendto_raw(a as Ri, (b as *const _, c), d, (e as *const _, f))
            }
            SyscallTable::SysSockRecvFrom => {
                sockets::syssock_recv_from_raw(a as Ri, (b as *mut _, c), d, e as *mut _)
            }
        }
    }

    // maps the results to an ErrorStatus
    let results = inner(number, a, b, c, d, e, f);
    let value = match results {
        Err(ErrorStatus::ForceTerminated) => {
            crate::thread::current::exit(ErrorStatus::ForceTerminated as usize)
        }
        Err(e) => SysResult::err(e),
        Ok(val) => SysResult::ok(val),
    };
    value
}
