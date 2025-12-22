use crate::drivers::vfs::SeekOffset;
use crate::memory::paging::EntryFlags;
use crate::memory::paging::MapToError;
use crate::process;
use crate::process::resources;
use crate::shared_mem;
use crate::shared_mem::ShmKey;
use crate::syscalls::ErrorStatus;
use crate::syscalls::SyscallFFI;
use macros::syscall_handler;
use safa_abi::mem::MemMapFlags;
use safa_abi::mem::RawMemMapConfig;
use safa_abi::mem::ShmFlags;

use crate::{VirtAddr, process::resources::Ri};

impl SyscallFFI for MemMapFlags {
    type Args = usize;
    #[inline(always)]
    fn make(args: Self::Args) -> Result<Self, safa_abi::errors::ErrorStatus> {
        Ok(MemMapFlags::from_bits_retaining(args as u8))
    }
}

#[syscall_handler]
pub fn sysmem_map(
    mmap_config: &RawMemMapConfig,
    flags: MemMapFlags,
    out_res_id: Option<&mut Ri>,
) -> Result<*mut u8, ErrorStatus> {
    if flags.contains(MemMapFlags::FIXED) {
        todo!("Fixed Mappings are not yet implemented")
    }

    let page_count = mmap_config.page_count;
    let guard_pages_count = mmap_config.guard_pages_count;
    let addr_hint = if mmap_config.addr_hint.is_null() {
        None
    } else {
        Some(VirtAddr::from_ptr(mmap_config.addr_hint))
    };

    let (associated_resource, resource_off) = if flags.contains(MemMapFlags::MAP_RESOURCE) {
        (
            Some(mmap_config.resource_to_map as Ri),
            Some(SeekOffset::from(mmap_config.resource_off)),
        )
    } else {
        (None, None)
    };

    let resource_off = resource_off.unwrap_or(SeekOffset::Start(0));

    let interface = associated_resource.map(|ri| {
        resources::get_ref(ri, |res| {
            res.data().open_mmap_interface(resource_off, page_count)
        })
        .ok_or(ErrorStatus::UnknownResource)
        .flatten()
    });

    let interface = match interface {
        Some(s) => Some(s?), /* ?????? */
        None => None,
    };

    let mut mem_flags = EntryFlags::USER_ACCESSIBLE;
    if flags.contains(MemMapFlags::WRITE) {
        mem_flags |= EntryFlags::WRITE;
    }

    if flags.contains(MemMapFlags::DISABLE_EXEC) {
        mem_flags |= EntryFlags::DISABLE_EXEC;
    }

    let curr_proc = process::current();
    let mut vasa = curr_proc.vasa();
    let tracker = vasa.map_n_pages_tracked_interface(
        addr_hint,
        page_count,
        guard_pages_count,
        mem_flags,
        interface,
    )?;

    let start_addr = tracker.start();
    // TODO: Implement local option
    let ri = resources::add_global_resource(tracker);

    if let Some(p) = out_res_id {
        *p = ri;
    }

    Ok(start_addr.into_ptr())
}

impl SyscallFFI for ShmFlags {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(ShmFlags::from_bits_retaining(args as u32))
    }
}

#[syscall_handler]
fn sysshm_create(
    pages_count: usize,
    _flags: ShmFlags,
    out_shm_key: &mut ShmKey,
) -> Result<Ri, MapToError> {
    let tracked_key =
        shared_mem::create_shm(pages_count).map_err(|()| MapToError::FrameAllocationFailed)?;
    let key = *tracked_key.key();

    let resource = tracked_key;
    let ri = resources::add_global_resource(resource);

    *out_shm_key = key;

    Ok(ri)
}

impl SyscallFFI for ShmKey {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(Self(args))
    }
}
#[syscall_handler]
fn sysshm_open(key: ShmKey, _flags: ShmFlags) -> Result<Ri, ErrorStatus> {
    let tracked_key = shared_mem::track_shm(key).ok_or(ErrorStatus::UnknownResource)?;

    let resource = tracked_key;
    let ri = resources::add_global_resource(resource);

    Ok(ri)
}
