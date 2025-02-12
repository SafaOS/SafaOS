use crate::{
    arch::power,
    debug,
    drivers::vfs::expose::{DirEntry, DirIter, DirIterRef, File, FileRef},
    threading::expose::SpawnFlags,
    utils::errors::ErrorStatus,
    VirtAddr,
};
use int_enum::IntEnum;
// TODO: make a proc-macro that generates the syscalls from rust functions
// for example it should generate a pointer and a length from a slice argument checking if it is vaild and
// returning invaild ptr if it is not
// it should also support optional pointer-arguments using Option<T>
// and we should do something about functions that takes a struct
mod io;
mod processes;
mod utils;

/// Safely converts FFI [`Self::Args`] into [`Self`] for being passed to a syscall
trait SyscallFFI: Sized {
    type Args;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus>;
}

/// converts `*const T` into `None` if the pointer is null if it is not aligned it will return an
/// [`ErrorStatus::InvaildPtr`]
impl<T> SyscallFFI for Option<&T> {
    type Args = *const T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() {
            Ok(None)
        } else if !args.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
        } else {
            Ok(unsafe { Some(&*args) })
        }
    }
}

/// converts `*mut T` into `None` if the pointer is null if it is not aligned it will return an
/// [`ErrorStatus::InvaildPtr`]
impl<T> SyscallFFI for Option<&mut T> {
    type Args = *mut T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() {
            Ok(None)
        } else if !args.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
        } else {
            Ok(unsafe { Some(&mut *args) })
        }
    }
}

impl<T> SyscallFFI for Option<&[T]> {
    type Args = (*const T, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        let slice = <&[T]>::make((ptr, len))?;
        if slice.is_empty() {
            Ok(None)
        } else {
            Ok(Some(slice))
        }
    }
}

impl SyscallFFI for Option<&str> {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        let opt = <Option<&[u8]>>::make((ptr, len))?;

        if let Some(slice) = opt {
            let str = core::str::from_utf8(slice).map_err(|_| ErrorStatus::InvaildStr)?;
            Ok(Some(str))
        } else {
            Ok(None)
        }
    }
}

/// converts `&T` into `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &T {
    type Args = *const T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() || !args.is_aligned() {
            Err(ErrorStatus::InvaildPtr)
        } else {
            Ok(unsafe { &*args })
        }
    }
}

/// converts `&mut T` into `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &mut T {
    type Args = *mut T;

    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        if args.is_null() || !args.is_aligned() {
            Err(ErrorStatus::InvaildPtr)
        } else {
            Ok(unsafe { &mut *args })
        }
    }
}

/// for an `&[T]` it will return `Err` if the pointer is null or not aligned
impl<T> SyscallFFI for &[T] {
    type Args = (*const T, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        if ptr.is_null() {
            Ok(&[])
        } else if !ptr.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
        } else {
            Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
        }
    }
}

impl<T> SyscallFFI for &mut [T] {
    type Args = (*mut T, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let (ptr, len) = args;
        if ptr.is_null() {
            Ok(&mut [])
        } else if !ptr.is_aligned() {
            return Err(ErrorStatus::InvaildPtr);
        } else {
            Ok(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
        }
    }
}

impl SyscallFFI for &str {
    type Args = (*const u8, usize);
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        let slice: &[u8] = SyscallFFI::make(args)?;
        core::str::from_utf8(slice).map_err(|_| ErrorStatus::InvaildPtr)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntEnum)]
#[repr(u16)]
pub enum SyscallTable {
    SysExit = 0,
    SysYield = 1,

    SysOpen = 2,
    SysDirIterOpen = 8,
    SysClose = 5,
    SysDirIterClose = 9,
    SysDirIterNext = 10,
    SysWrite = 3,
    SysRead = 4,
    SysCreate = 6,
    SysCreateDir = 7,
    SysSync = 16,
    SysTruncate = 17,

    SysCHDir = 14,
    SysGetCWD = 15,
    SysSbrk = 18,

    SysPSpawn = 19,
    SysWait = 11,

    SysShutdown = 20,
    SysReboot = 21,
}

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
                let path = <&str>::make((a as *const u8, b))?;
                utils::syschdir(path)
            }
            // io
            SyscallTable::SysOpen => {
                let path = <&str>::make((a as *const u8, b))?;
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
                let path = <&str>::make((a as *const u8, b))?;
                io::syscreate(path)
            }
            SyscallTable::SysCreateDir => {
                let path = <&str>::make((a as *const u8, b))?;
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

                let path = <&str>::make((a as *const u8, b))?;
                let config = <&SpawnConfig>::make(c as *const SpawnConfig)?;
                let (name, argv, flags) = config.as_rust()?;
                let dest_pid = Option::make(d as *mut usize)?;

                processes::syspspawn(name, path, argv, flags, dest_pid)
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
