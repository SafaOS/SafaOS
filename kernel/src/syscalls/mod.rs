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
// TODO: make a proc-macro that generates the syscalls from rust functions
// for example it should generate a pointer and a length from a slice argument checking if it is vaild and
// returning invaild ptr if it is not
// it should also support optional pointer-arguments using Option<T>
// and we should do something about functions that takes a struct
mod io;
mod processes;
mod utils;

#[inline(always)]
/// takes the number of the syscall and the arguments and returns an error as a u16 if it fails
/// this function is the final non-arch-specific layer between the kernel and the syscalls
/// it maps from arguments to syscall arguments
/// the way arguments are mapped is defined by the [`SyscallFFI`] trait
pub fn syscall(number: u16, a: usize, b: usize, c: usize, d: usize, e: usize) -> ErrorStatus {
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
            // processes
            SyscallTable::SysPSpawn => {
                #[inline(always)]
                /// converts slice of raw pointers to a slice of strs which is used by pspawn as
                /// process arguments
                fn into_args_slice<'a>(
                    args_raw: *mut (*const u8, usize),
                    len: usize,
                ) -> Result<&'a [&'a str], ErrorStatus> {
                    if len == 0 {
                        return Ok(&[]);
                    }

                    let raw_slice: &mut [(*const u8, usize)] = SyscallFFI::make((args_raw, len))?;
                    // unsafely creates a muttable reference to `raw_slice`
                    let double_slice: &mut [&str] =
                        unsafe { &mut *(raw_slice as *const _ as *mut [&str]) };

                    // maps every parent_slice[i] to &str
                    for (i, item) in raw_slice.iter().enumerate() {
                        double_slice[i] = <&str>::make(*item)?;
                    }

                    Ok(double_slice)
                }

                /// the temporary config struct for the spawn syscall, passed to the syscall
                /// because if it was passed as a bunch of arguments it would be too big to fit
                /// inside the registers
                #[repr(C)]
                struct SpawnConfig {
                    name: (*const u8, usize),
                    argv: (*mut (*const u8, usize), usize),
                    flags: SpawnFlags,
                }

                impl SpawnConfig {
                    fn as_rust(&self) -> Result<(Option<&str>, &[&str], SpawnFlags), ErrorStatus> {
                        let name = Option::<&str>::make((self.name.0, self.name.1))?;
                        let argv = into_args_slice(self.argv.0, self.argv.1)?;

                        Ok((name, argv, self.flags))
                    }
                }

                let path = <Path>::make((a as *const u8, b))?;
                let config = <&SpawnConfig>::make(c as *const SpawnConfig)?;
                let (name, argv, flags) = config.as_rust()?;
                let dest_pid = Option::make(d as *mut usize)?;

                processes::syspspawn(name, path, argv, flags, dest_pid)
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
    let value = match inner(number, a, b, c, d, e) {
        Ok(()) => ErrorStatus::None,
        Err(err) => err,
    };

    value
}
