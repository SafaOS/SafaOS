use super::ffi::SyscallFFI;
use crate::{
    drivers::vfs::{CollectionIterDescriptor, FSObjectDescriptor, SeekOffset},
    process::resources::{self, Ri},
    utils::locks::Mutex,
    vtty,
};

use macros::syscall_handler;
use safa_abi::{
    errors::ErrorStatus,
    fs::{DirEntry, FileAttr},
};

#[syscall_handler]
fn syswrite(
    fd: Ri,
    offset: isize,
    buf: &[u8],
    dest_wrote: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let off = SeekOffset::from(offset);

    let resource = resources::get_expected(fd)?;
    let wrote = resource.data().write(off, buf)?;

    if let Some(dest_wrote) = dest_wrote {
        *dest_wrote = wrote;
    }

    Ok(())
}

#[syscall_handler]
fn sysread(
    fd: Ri,
    offset: isize,
    buf: &mut [u8],
    dest_read: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let off = SeekOffset::from(offset);
    let resource = resources::get_expected(fd)?;
    let bytes_read = resource.data().read(off, buf)?;

    if let Some(dest_read) = dest_read {
        *dest_read = bytes_read;
    }

    Ok(())
}

#[syscall_handler]
fn sysdiriter_open(dir_rd: Ri, dest_diriter: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let resource = resources::get(dir_rd).ok_or(ErrorStatus::UnknownResource)?;
    let fd = resource.data().as_ref_expected::<FSObjectDescriptor>()?;
    let diriter = fd.open_collection_iter()?;

    let ri = resources::add_global_resource(Mutex::new(diriter));
    if let Some(dest_diriter) = dest_diriter {
        *dest_diriter = ri;
    }
    Ok(())
}

#[syscall_handler]
fn sysdiriter_next(diriter_rd: Ri, direntry: &mut DirEntry) -> Result<(), ErrorStatus> {
    let resource = resources::get_expected(diriter_rd)?;
    let diriter = resource
        .data()
        .as_ref_expected::<Mutex<CollectionIterDescriptor>>()?;

    let next = diriter.lock().next();
    if let Some(next) = next {
        *direntry = next;
        Ok(())
    } else {
        *direntry = unsafe { core::mem::zeroed() };
        Err(ErrorStatus::Generic)
    }
}

#[syscall_handler]
fn syssync(ri: Ri) -> Result<(), ErrorStatus> {
    let resource = resources::get_expected(ri)?;
    resource.data().sync()
}

#[syscall_handler]
fn systruncate(fd: Ri, len: usize) -> Result<(), ErrorStatus> {
    let resource = resources::get_expected(fd)?;
    resource.data().truncate(len)
}

// TODO: add always successful syscall handlers support
#[syscall_handler]
fn sysfsize(ri: Ri, dest_fd: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let resource = resources::get_expected(ri)?;
    let fd = resource.data().as_ref_expected::<FSObjectDescriptor>()?;
    if let Some(dest_fd) = dest_fd {
        *dest_fd = fd.size();
    }
    Ok(())
}

#[syscall_handler]
fn sysattrs(ri: Ri, dest_attrs: Option<&mut FileAttr>) -> Result<(), ErrorStatus> {
    let resource = resources::get_expected(ri)?;
    let fd = resource.data().as_ref_expected::<FSObjectDescriptor>()?;
    if let Some(dest_attrs) = dest_attrs {
        *dest_attrs = fd.attrs();
    }
    Ok(())
}

#[syscall_handler]
fn sysdup(resource: Ri, dest_resource: &mut Ri) -> Result<(), ErrorStatus> {
    *dest_resource = resources::duplicate_resource(resource)
        .ok_or(ErrorStatus::UnknownResource)
        .flatten()?;
    Ok(())
}

#[syscall_handler]
fn sysio_command(ri: Ri, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
    let resource = resources::get_expected(ri)?;
    resource.data().send_command(cmd, arg)
}

#[syscall_handler]
fn sysvtty_alloc(mother_ri: &mut Ri, child_ri: &mut Ri) {
    let (mother, child) = vtty::alloc_vtty();
    let m = resources::add_global_resource(mother);
    let c = resources::add_global_resource(child);

    *mother_ri = m;
    *child_ri = c;
}
