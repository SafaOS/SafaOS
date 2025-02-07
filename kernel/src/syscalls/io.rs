use crate::{
    drivers::vfs::{
        self,
        expose::{DirIterRef, FileRef},
    },
    utils::errors::ErrorStatus,
};

pub fn sysopen(path: &str, dest_fd: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let file_ref = FileRef::open(path)?;
    if let Some(dest_fd) = dest_fd {
        *dest_fd = file_ref.ri();
    }

    Ok(())
}

pub fn syswrite(
    fd: FileRef,
    offset: isize,
    buf: &[u8],
    dest_wrote: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let bytes_wrote = fd.write(offset, buf)?;
    if let Some(dest_wrote) = dest_wrote {
        *dest_wrote = bytes_wrote;
    }

    Ok(())
}

pub fn sysread(
    fd: FileRef,
    offset: isize,
    buf: &mut [u8],
    dest_read: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let bytes_read = fd.read(offset, buf)?;
    if let Some(dest_read) = dest_read {
        *dest_read = bytes_read;
    }

    Ok(())
}

pub fn syscreate(path: &str) -> Result<(), ErrorStatus> {
    vfs::expose::create(path).map_err(|err| err.into())
}

pub fn syscreatedir(path: &str) -> Result<(), ErrorStatus> {
    vfs::expose::createdir(path).map_err(|err| err.into())
}

pub fn sysdiriter_open(
    dir_rd: FileRef,
    dest_diriter: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let diriter = dir_rd.diriter_open()?;
    if let Some(dest_diriter) = dest_diriter {
        *dest_diriter = diriter.ri();
    }
    Ok(())
}

pub fn sysdiriter_next(
    diriter_rd: DirIterRef,
    direntry: &mut vfs::expose::DirEntry,
) -> Result<(), ErrorStatus> {
    let next = diriter_rd.next();
    if let Some(next) = next {
        *direntry = next;
        Ok(())
    } else {
        *direntry = unsafe { vfs::expose::DirEntry::zeroed() };
        Err(ErrorStatus::Generic)
    }
}

pub fn syssync(fd: FileRef) -> Result<(), ErrorStatus> {
    fd.sync().map_err(|e| e.into())
}

pub fn systruncate(fd: FileRef, len: usize) -> Result<(), ErrorStatus> {
    fd.truncate(len).map_err(|e| e.into())
}
