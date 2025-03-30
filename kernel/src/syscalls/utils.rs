use alloc::format;

use crate::{
    threading,
    utils::{errors::ErrorStatus, path::Path},
    VirtAddr,
};

pub fn sysexit(code: usize) -> ! {
    threading::expose::thread_exit(code)
}

pub fn sysyield() {
    threading::expose::thread_yeild()
}

pub fn syschdir(path: Path) -> Result<(), ErrorStatus> {
    threading::expose::chdir(path).map_err(|err| err.into())
}
/// gets the current working directory in `path` and puts the length of the gotten path in
/// `dest_len` if it is not null
/// returns ErrorStatus::Generic if the path is too long to fit in the given buffer `path`
pub fn sysgetcwd(path: &mut [u8], dest_len: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let state = threading::this_state();
    let cwd = state.cwd();
    let len = cwd.len();

    if len > path.len() {
        return Err(ErrorStatus::Generic);
    }

    path[..len].copy_from_slice(format!("{cwd}").as_bytes());

    if let Some(dest_len) = dest_len {
        *dest_len = len;
    }

    Ok(())
}

// on fail returns null for unknown reasons
pub fn syssbrk(amount: isize, results: &mut VirtAddr) -> Result<(), ErrorStatus> {
    let res = threading::expose::sbrk(amount)?;
    *results = res as VirtAddr;
    Ok(())
}
