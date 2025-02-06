// TODO: re-design this module
use crate::{
    drivers::vfs::{
        self,
        expose::{DirIter, DirIterRef, File, FileRef},
    },
    utils::{
        errors::ErrorStatus,
        ffi::{Optional, RequiredMut, Slice, SliceMut},
    },
};

#[no_mangle]
extern "C" fn sysopen(path_ptr: *const u8, len: usize, dest_fd: Optional<usize>) -> ErrorStatus {
    let path = Slice::new(path_ptr, len)?.into_str();

    match FileRef::open(path) {
        Ok(file_ref) => {
            if let Some(dest_fd) = dest_fd.into_option() {
                *dest_fd = file_ref.ri();
            }
            ErrorStatus::None
        }
        Err(err) => err.into(),
    }
}

#[no_mangle]
extern "C" fn syswrite(
    fd: usize,
    offset: isize,
    ptr: *const u8,
    len: usize,
    dest_wrote: Optional<usize>,
) -> ErrorStatus {
    let slice = Slice::new(ptr, len)?.into_slice();
    let file_ref = FileRef::get(fd).ok_or(ErrorStatus::InvaildResource)?;

    let bytes_wrote = file_ref.write(offset, slice).map_err(|err| err.into())?;
    if let Some(dest_wrote) = dest_wrote.into_option() {
        *dest_wrote = bytes_wrote;
    }

    ErrorStatus::None
}

#[no_mangle]
extern "C" fn sysread(
    fd: usize,
    offset: isize,
    ptr: *mut u8,
    len: usize,
    dest_read: Optional<usize>,
) -> ErrorStatus {
    let slice = SliceMut::new(ptr, len)?.into_slice();
    let file_ref = FileRef::get(fd).ok_or(ErrorStatus::InvaildResource)?;

    let bytes_read = file_ref.read(offset, slice).map_err(|err| err.into())?;
    if let Some(dest_read) = dest_read.into_option() {
        *dest_read = bytes_read;
    }

    ErrorStatus::None
}

#[no_mangle]
extern "C" fn sysclose(fd: usize) -> ErrorStatus {
    let _ = File::from_fd(fd).ok_or(ErrorStatus::InvaildResource)?;
    ErrorStatus::None
}

#[no_mangle]
extern "C" fn syscreate(path_ptr: *const u8, path_len: usize) -> ErrorStatus {
    let path = Slice::new(path_ptr, path_len)?.into_str();

    if let Err(err) = vfs::expose::create(path) {
        err.into()
    } else {
        ErrorStatus::None
    }
}

#[no_mangle]
extern "C" fn syscreatedir(path_ptr: *const u8, path_len: usize) -> ErrorStatus {
    let path = Slice::new(path_ptr, path_len)?.into_str();

    if let Err(err) = vfs::expose::createdir(path) {
        err.into()
    } else {
        ErrorStatus::None
    }
}

#[no_mangle]
extern "C" fn sysdiriter_open(dir_ri: usize, dest_diriter: Optional<usize>) -> ErrorStatus {
    let file_ref = FileRef::get(dir_ri).ok_or(ErrorStatus::InvaildResource)?;

    match file_ref.diriter_open() {
        Err(err) => err.into(),
        Ok(dir) => {
            if let Some(dest_diriter) = dest_diriter.into_option() {
                *dest_diriter = dir.ri();
            }
            ErrorStatus::None
        }
    }
}

#[no_mangle]
extern "C" fn sysdiriter_close(diriter_ri: usize) -> ErrorStatus {
    let _ = DirIter::from_ri(diriter_ri).ok_or(ErrorStatus::InvaildResource)?;
    ErrorStatus::None
}

#[no_mangle]
extern "C" fn sysdiriter_next(
    diriter_ri: usize,
    direntry_ptr: RequiredMut<vfs::expose::DirEntry>,
) -> ErrorStatus {
    let diriter_ref = DirIterRef::get(diriter_ri).ok_or(ErrorStatus::InvaildResource)?;
    let direntry_ref = direntry_ptr.get()?;

    match diriter_ref.next() {
        None => {
            *direntry_ref = unsafe { vfs::expose::DirEntry::zeroed() };
            ErrorStatus::Generic
        }
        Some(direntry) => {
            *direntry_ref = direntry;
            ErrorStatus::None
        }
    }
}

#[no_mangle]
extern "C" fn syssync(ri: usize) -> ErrorStatus {
    let file_ref = FileRef::get(ri).ok_or(ErrorStatus::InvaildResource)?;

    file_ref.sync().map_err(|e| e.into())?;
    ErrorStatus::None
}

#[no_mangle]
extern "C" fn systruncate(ri: usize, len: usize) -> ErrorStatus {
    let file_ref = FileRef::get(ri).ok_or(ErrorStatus::InvaildResource)?;

    file_ref.truncate(len).map_err(|e| e.into())?;
    ErrorStatus::None
}
