use core::fmt::Write;
use macros::syscall_handler;
use safa_utils::io::Cursor;

use super::SyscallFFI;
use crate::{
    threading,
    utils::{errors::ErrorStatus, path::Path},
    VirtAddr,
};

pub fn sysexit(code: usize) -> ! {
    threading::expose::thread_exit(code)
}

pub fn sysyield() {
    threading::expose::thread_yield()
}

#[syscall_handler]
fn syschdir(path: Path) -> Result<(), ErrorStatus> {
    threading::expose::chdir(path).map_err(|err| err.into())
}

#[syscall_handler]
/// gets the current working directory in `path` and puts the length of the gotten path in
/// `dest_len` if it is not null
/// returns ErrorStatus::Generic if the path is too long to fit in the given buffer `path`
fn sysgetcwd(path: &mut [u8], dest_len: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let state = threading::this_state();
    let cwd = state.cwd();

    let len = cwd.len();
    if let Some(dest_len) = dest_len {
        *dest_len = len;
    }

    let mut cursor = Cursor::new(path);
    write!(&mut cursor, "{cwd}").map_err(|_| ErrorStatus::Generic)?;
    Ok(())
}

#[syscall_handler]
fn syssbrk(amount: isize, results: &mut VirtAddr) -> Result<(), ErrorStatus> {
    let res = threading::expose::sbrk(amount)?;
    *results = VirtAddr::from_ptr(res);
    Ok(())
}
