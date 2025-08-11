use crate::drivers::vfs::SeekOffset;
use crate::fs::FileRef;
use crate::memory::paging::EntryFlags;
use crate::process;
use crate::process::resources;
use crate::process::resources::ResourceData;
use crate::syscalls::ErrorStatus;
use crate::syscalls::SyscallFFI;
use alloc::sync::Arc;
use macros::syscall_handler;
use safa_abi::mem::MemMapFlags;
use safa_abi::mem::RawMemMapConfig;

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
    out_start_addr: Option<&mut VirtAddr>,
) -> Result<(), ErrorStatus> {
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
            Some(mmap_config.resource_to_map),
            Some(SeekOffset::from(mmap_config.resource_off)),
        )
    } else {
        (None, None)
    };

    let file_desc = match associated_resource {
        Some(ri) => {
            let file = FileRef::make(ri)?;
            Some(file.with_fd(|f| f.clone()))
        }
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
    let tracker = vasa.map_n_pages_tracked(
        addr_hint,
        page_count,
        guard_pages_count,
        mem_flags,
        core::iter::empty(),
        file_desc,
        resource_off,
    )?;

    let start_addr = tracker.start();
    // TODO: Implement local option
    let ri = resources::add_global_resource(ResourceData::TrackedMapping(Arc::new(tracker)));

    if let Some(p) = out_res_id {
        *p = ri;
    }

    if let Some(p) = out_start_addr {
        *p = start_addr;
    }

    Ok(())
}
