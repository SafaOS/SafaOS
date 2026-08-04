use crate::drivers::vfs::SeekOffset;
use crate::memory::paging::MapToError;
use crate::memory::paging::PAGE_SIZE;
use crate::memory::vmm;
use crate::memory::vmm::Location;
use crate::memory::vmm::VMMMFlags;
use crate::process;
use crate::process::mem::TrackedMemoryAllocation;
use crate::process::resources;
use crate::shared_mem;
use crate::shared_mem::ShmKey;
use crate::syscalls::ErrorStatus;
use crate::syscalls::SyscallFFI;
use crate::syscalls::ffi::ExpectedResource;
use macros::syscall_handler;
use safa_abi::mem::MemFlags;
use safa_abi::mem::MemMap2Flags;
use safa_abi::mem::MemMapFlags;
use safa_abi::mem::MemMapOp;
use safa_abi::mem::MemMapTarget;
use safa_abi::mem::RawMemMap2Config;
use safa_abi::mem::RawMemMapConfig;
use safa_abi::mem::ShmFlags;

use crate::{VirtAddr, process::resources::Ri};

impl SyscallFFI for MemMapFlags {
    type Args = usize;
    #[inline(always)]
    fn make(args: Self::Args) -> Result<Self, safa_abi::errors::ErrorStatus> {
        Ok(MemMapFlags::from_bits(args as u8))
    }
}

impl SyscallFFI for MemMap2Flags {
    type Args = usize;
    #[inline(always)]
    fn make(args: Self::Args) -> Result<Self, safa_abi::errors::ErrorStatus> {
        Ok(MemMap2Flags::from_bits(args as u32))
    }
}

impl SyscallFFI for MemFlags {
    type Args = usize;
    #[inline(always)]
    fn make(args: Self::Args) -> Result<Self, safa_abi::errors::ErrorStatus> {
        Ok(MemFlags::from_bits(args as u8))
    }
}

#[syscall_handler]
pub fn sysmem_protect(
    resource: ExpectedResource<TrackedMemoryAllocation>,
    flags: u16,
) -> Result<(), ErrorStatus> {
    let flags = MemFlags::from_bits(flags as u8);
    let addr = resource.start();

    let mut vmm_flags = VMMMFlags::USER_ACCESSIBLE;
    if flags.contains(MemFlags::WRITE) {
        vmm_flags |= VMMMFlags::WRITABLE;
    }

    if flags.contains(MemFlags::EXEC) {
        vmm_flags |= VMMMFlags::EXECUTABLE;
    }

    vmm::with_user_vmm(|vmm| {
        assert!(
            vmm.set_page_flags(addr, vmm_flags),
            "Valid memory Resource at {:?} corrupted, failed to sysmem_protect.",
            resource.start(),
        )
    });
    Ok(())
}

#[syscall_handler]
pub fn sysmem_map(
    mmap_config: &RawMemMapConfig,
    flags: MemMapFlags,
    out_res_id: Option<&mut Ri>,
) -> Result<*mut u8, ErrorStatus> {
    let page_count = mmap_config.page_count;
    // TODO: Implement guard pages
    // let guard_pages_count = mmap_config.guard_pages_count;
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

    if associated_resource.is_some() && out_res_id.is_none() {
        return Err(ErrorStatus::InvalidArgument);
    }

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

    let mut mem_flags = VMMMFlags::empty();
    if flags.contains(MemMapFlags::WRITE) {
        mem_flags |= VMMMFlags::WRITABLE;
    }

    if !flags.contains(MemMapFlags::DISABLE_EXEC) {
        mem_flags |= VMMMFlags::EXECUTABLE;
    }

    let location = addr_hint.map(|s| {
        if flags.contains(MemMapFlags::FIXED) {
            Location::Fixed(s)
        } else {
            Location::Hint(s)
        }
    });
    let tracker = process::mem::mem_map(
        location,
        page_count,
        mem_flags,
        flags.contains(MemMapFlags::POPULATE),
        interface,
    )?;
    let start_addr = tracker.start();

    if let Some(p) = out_res_id {
        let ri = resources::add_global_resource(tracker);
        *p = ri;
    } else {
        core::mem::forget(tracker);
    }

    Ok(start_addr.into_ptr())
}

#[syscall_handler]
pub fn sysmem_map2(
    mmap_config: &RawMemMap2Config,
    flags: MemMap2Flags,
    prot: MemFlags,
    out_res_id: Option<&mut Ri>,
) -> Result<*mut u8, ErrorStatus> {
    let fixed = flags.contains(MemMap2Flags::FIXED);

    let page_count = mmap_config.size_pages(PAGE_SIZE);
    let addr_location = if mmap_config.addr_hint().is_null() {
        None
    } else {
        let addr = VirtAddr::from_ptr(mmap_config.addr_hint());
        Some(if fixed {
            Location::Fixed(addr)
        } else {
            Location::Hint(addr)
        })
    };

    let (associated_resource, resource_off) = if flags.contains(MemMap2Flags::MAP_RESOURCE) {
        (
            Some(mmap_config.resource().0),
            Some(SeekOffset::from(mmap_config.resource().1)),
        )
    } else {
        (None, None)
    };

    if associated_resource.is_some() && out_res_id.is_none() {
        return Err(ErrorStatus::InvalidArgument);
    }

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

    let mut mem_flags = VMMMFlags::empty();
    if prot.contains(MemFlags::WRITE) {
        mem_flags |= VMMMFlags::WRITABLE;
    }

    if prot.contains(MemFlags::EXEC) {
        mem_flags |= VMMMFlags::EXECUTABLE;
    }

    let tracker = process::mem::mem_map(
        addr_location,
        page_count,
        mem_flags,
        flags.contains(MemMap2Flags::POPULATE),
        interface,
    )?;
    let start_addr = tracker.start();

    if let Some(p) = out_res_id {
        let ri = resources::add_global_resource(tracker);
        *p = ri;
    } else {
        core::mem::forget(tracker);
    }

    Ok(start_addr.into_ptr())
}

#[syscall_handler]
pub fn sysmem_op(target: &MemMapTarget, op: &MemMapOp) -> Result<(), ErrorStatus> {
    let (addr, size) = match target {
        MemMapTarget::Resource(ri) => (
            ExpectedResource::<TrackedMemoryAllocation>::make(*ri)?.start(),
            None,
        ),
        MemMapTarget::Direct(addr, size) => {
            if !(*addr as usize).is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
                return Err(ErrorStatus::InvalidArgument);
            }

            (VirtAddr::from_ptr(*addr), Some(size))
        }
    };

    match op {
        MemMapOp::Protect(prot) => {
            let mut vmm_flags = VMMMFlags::USER_ACCESSIBLE;
            if prot.contains(MemFlags::WRITE) {
                vmm_flags |= VMMMFlags::WRITABLE;
            }

            if prot.contains(MemFlags::EXEC) {
                vmm_flags |= VMMMFlags::EXECUTABLE;
            }

            vmm::with_user_vmm(|vmm| {
                if let Some(size) = size {
                    if !vmm.set_page_flags_contiguous(addr, *size, vmm_flags) {
                        return Err(ErrorStatus::MMapError);
                    }
                } else {
                    if !vmm.set_page_flags(addr, vmm_flags) {
                        return Err(ErrorStatus::MMapError);
                    }
                }
                Ok(())
            })?;

            Ok(())
        }
    }
}

#[syscall_handler]
pub fn sysmem_unmap(addr: usize, size: usize) -> Result<(), ErrorStatus> {
    if !addr.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
        return Err(ErrorStatus::InvalidArgument);
    }

    let addr = VirtAddr::from(addr);

    vmm::with_user_vmm(|vmm| {
        if !vmm.unmap_contiugous(addr, size) {
            Err(ErrorStatus::MMapError)
        } else {
            Ok(())
        }
    })
}

impl SyscallFFI for ShmFlags {
    type Args = usize;
    fn make(args: Self::Args) -> Result<Self, ErrorStatus> {
        Ok(ShmFlags::from_bits(args as u32))
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
