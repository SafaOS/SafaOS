use crate::{
    threading,
    utils::{
        self,
        errors::ErrorStatus,
        expose::SysInfo,
        ffi::{Optional, RequiredMut, Slice, SliceMut},
    },
};

/// for now
#[no_mangle]
extern "C" fn sysexit(code: usize) {
    threading::expose::thread_exit(code);
}

#[no_mangle]
extern "C" fn sysyield() {
    threading::expose::thread_yeild()
}

#[no_mangle]
extern "C" fn syschdir(path_ptr: *const u8, path_len: usize) -> ErrorStatus {
    let path = Slice::new(path_ptr, path_len)?.into_str();

    if let Err(err) = threading::expose::chdir(path) {
        err.into()
    } else {
        ErrorStatus::None
    }
}

#[no_mangle]
extern "C" fn sysgetcwd(path_ptr: *mut u8, len: usize, dest_len: Optional<usize>) -> ErrorStatus {
    let path = SliceMut::new(path_ptr, len)?.into_slice();
    let got = threading::expose::getcwd().into_bytes();

    if got.len() > len {
        return ErrorStatus::Generic;
    }

    path[..got.len()].copy_from_slice(&got);

    if let Some(dest_len) = dest_len.into_option() {
        *dest_len = got.len();
    }

    ErrorStatus::None
}

// on fail returns null for unknown reasons
#[no_mangle]
extern "C" fn syssbrk(amount: isize) -> *mut u8 {
    threading::expose::sbrk(amount)
}

#[no_mangle]
extern "C" fn sysinfo(ptr: RequiredMut<SysInfo>) -> ErrorStatus {
    utils::expose::info(ptr.get()?);

    ErrorStatus::None
}
