use super::ffi::SyscallFFI;
use crate::{
    drivers::vfs::{CollectionIterDescriptor, FSObjectDescriptor, SeekOffset},
    process::{
        poll::{self, PollError},
        resources::{self, Ri},
    },
    syscalls::ffi::{ExpectedResource, ResourceDesc},
    utils::locks::Mutex,
    vtty,
};

use macros::syscall_handler;
use safa_abi::{
    errors::ErrorStatus,
    fs::{DirEntry, FileAttr},
};

#[syscall_handler]
fn syswrite(resource: ResourceDesc, offset: SeekOffset, buf: &[u8]) -> Result<usize, ErrorStatus> {
    resource.write(offset, buf)
}

#[syscall_handler]
fn sysread(
    resource: ResourceDesc,
    offset: SeekOffset,
    buf: &mut [u8],
) -> Result<usize, ErrorStatus> {
    resource.read(offset, buf)
}

#[syscall_handler]
fn sysdiriter_open(dir_rd: Ri) -> Result<Ri, ErrorStatus> {
    let resource = resources::get(dir_rd).ok_or(ErrorStatus::UnknownResource)?;
    let fd = resource.data().as_ref_expected::<FSObjectDescriptor>()?;
    let diriter = fd.open_collection_iter()?;

    let ri = resources::add_global_resource(Mutex::new(diriter));
    Ok(ri)
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
fn syssync(resource: ResourceDesc) -> Result<(), ErrorStatus> {
    resource.sync()
}

#[syscall_handler]
fn systruncate(resource: ResourceDesc, len: usize) -> Result<(), ErrorStatus> {
    resource.truncate(len)
}

#[syscall_handler]
fn sysfsize(fd: ExpectedResource<FSObjectDescriptor>) -> usize {
    fd.size()
}

#[syscall_handler]
fn sysattrs(fd: ExpectedResource<FSObjectDescriptor>, dest_attrs: Option<&mut FileAttr>) {
    if let Some(dest_attrs) = dest_attrs {
        *dest_attrs = fd.attrs();
    }
}

#[syscall_handler]
fn sysclone(resource: Ri) -> Result<Ri, ErrorStatus> {
    resources::duplicate_resource(resource)
        .ok_or(ErrorStatus::UnknownResource)
        .flatten()
}

#[syscall_handler]
fn sysio_command(resource: ResourceDesc, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
    resource.send_command(cmd, arg)
}

#[syscall_handler]
fn sysvtty_alloc(mother_ri: &mut Ri, child_ri: &mut Ri) {
    let (mother, child) = vtty::alloc_vtty();
    let m = resources::add_global_resource(mother);
    let c = resources::add_global_resource(child);

    *mother_ri = m;
    *child_ri = c;
}

#[syscall_handler]
fn sysio_poll(
    resources: &mut [safa_abi::poll::PollEntry],
    timeout_after: u64,
) -> Result<(), PollError> {
    poll::poll_resources(resources, timeout_after)
}
