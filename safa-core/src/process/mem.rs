//! Memory management utilities for processes.
use core::ptr::NonNull;

use alloc::{boxed::Box, sync::Arc};
use cfg_if::cfg_if;
use safa_abi::{ffi::slice::Slice, process::AbiStructures};

use crate::{
    VirtAddr,
    arch::without_interrupts,
    drivers::vfs::{FSError, FSResult},
    memory::{
        self, AlignTo, AlignToPage,
        frame_allocator::Frame,
        paging::{MapToError, PAGE_SIZE},
        vmm::{self, Location, VMMAllocError, VMMMFlags, VirtualMemoryManager, with_user_vmm},
    },
    process::resources::Resource,
    thread, warn,
};

/// A process memory allocation that is tracked, to be freed by the VMM when dropped.
///
/// Is a [`Resource`].
pub struct TrackedMemoryAllocation {
    addr: VirtAddr,
    vmm: Option<Arc<VirtualMemoryManager>>,
    interface: Option<Box<dyn MemMappedInterface>>,
}

impl core::fmt::Debug for TrackedMemoryAllocation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TrackedMemoryAllocation")
            .field("addr", &self.addr)
            .field("has_interface", &self.interface.is_some())
            .finish()
    }
}

impl Resource for TrackedMemoryAllocation {
    fn address_space_generic(&self) -> bool {
        false
    }

    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), safa_abi::errors::ErrorStatus> {
        if let Some(interface) = self.interface.as_ref() {
            Ok(interface.send_command(cmd, arg)?)
        } else {
            Err(safa_abi::errors::ErrorStatus::OperationNotSupported)
        }
    }
    fn sync(&self) -> Result<(), safa_abi::errors::ErrorStatus> {
        if let Some(interface) = self.interface.as_ref() {
            Ok(interface.sync()?)
        } else {
            Err(safa_abi::errors::ErrorStatus::OperationNotSupported)
        }
    }
}

impl TrackedMemoryAllocation {
    pub fn new(
        vmm: Option<Arc<VirtualMemoryManager>>,
        addr: VirtAddr,
        interface: Option<Box<dyn MemMappedInterface>>,
    ) -> Self {
        Self {
            vmm,
            addr,
            interface,
        }
    }

    /// Returns the starting address of the allocation.
    pub const fn start(&self) -> VirtAddr {
        self.addr
    }
}

impl Drop for TrackedMemoryAllocation {
    fn drop(&mut self) {
        let unmap_func = |vmm: &VirtualMemoryManager, addr: VirtAddr| {
            if !vmm.unmap(addr) {
                warn!(
                    TrackedMemoryAllocation,
                    "Failed to drop: {addr:?}, not mapped"
                );
            }
        };

        let unmap_addr = |addr: VirtAddr| {
            if let Some(vmm) = self.vmm.as_ref() {
                without_interrupts(|| unmap_func(vmm, addr))
            } else {
                vmm::with_root(|vmm| unmap_func(vmm, addr));
            }
        };

        if let Some(interface) = self.interface.as_ref() {
            _ = interface.sync();
        }
        unmap_addr(self.addr);
    }
}

/// Describes any memory-mapped device interface.
pub trait MemMappedInterface {
    fn frames(&self) -> &[Frame];
    fn sync(&self) -> FSResult<()> {
        Ok(())
    }

    fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        _ = cmd;
        _ = arg;
        Err(FSError::OperationNotSupported)
    }
}

/// Allocates the root thread stack and environment variables, arguments storage, and etc, then returns
/// (the allocation, (stack_end, env_vars_ptrs, args_ptrs, abi_structures_ptr)).
pub fn allocate_root_user_env(
    vmm: Arc<VirtualMemoryManager>,
    stack_size: usize,
    env: &[&[u8]],
    args: &[&str],
    abi_structures: AbiStructures,
) -> Result<
    (
        TrackedMemoryAllocation,
        (VirtAddr, VirtAddr, VirtAddr, VirtAddr),
    ),
    MapToError,
> {
    let stack_size = stack_size.to_next_multiple_of(0x10usize);
    let env_total_size = env.iter().map(|e| e.len()).sum::<usize>();
    let args_total_size = args.iter().map(|e| e.len()).sum::<usize>();

    // The root environment has N sections:
    // 1. Raw Environment Variables(Align 1)
    // 2. Raw Argument Variables   (Align 1)
    // 3. Environment Pointers     (Align 0x8)
    // 4. Argument Pointers        (Align 0x8)
    // 5. ABI Structures           (Align 0x8)
    // 6. Root thread stack        (Align 0x10)

    // ====================== Calculate sizes and offsets =========================
    // Total size of all the sections
    let mut raw_size = 0;
    let raw_slices_section_off = raw_size;

    let raw_env_section_size =
        size_of::<usize>() /* envc */ + env_total_size + (1 * env.len() /* null terminators */);
    let raw_args_section_start = raw_env_section_size;
    let raw_args_section_size =
        size_of::<usize>() /* argc */ + args_total_size + (1 * args.len() /* null terminators */);
    let raw_slices_section_size = raw_args_section_size + raw_env_section_size;

    // Pointers come after the raw slices section, with align_of::<Slice<u8>> alignment.
    raw_size += raw_slices_section_size.to_next_multiple_of(align_of::<Slice<u8>>());
    // Offset for environment pointers section
    let env_pointers_off = raw_size;

    // Each "pointer" is a Slice<u8>
    let env_pointers_section_size = env.len() * size_of::<Slice<u8>>();
    raw_size += env_pointers_section_size;
    // Offset for argument pointers section
    let args_pointers_off = raw_size;
    let args_pointers_section_size = args.len() * size_of::<Slice<u8>>();
    raw_size += args_pointers_section_size;
    // Offset for ABI structures section
    let abi_structures_off = raw_size;
    raw_size += size_of::<AbiStructures>();
    // Offset for stack section
    let stack_off = raw_size.to_next_multiple_of(0x10usize);
    raw_size += stack_size;
    // ==================   End Calculations   =======================
    // ================== Allocation & Copying =======================
    let allocation_size = raw_size.to_next_page();
    let addr = allocate_user_stuff(&"thread.root_stack", &vmm, allocation_size)?;
    let vmm_table = unsafe { &mut *vmm.table_ptr() };

    let mut current = raw_slices_section_off;
    /// Copy bytes to the vmm's table

    macro_rules! copy_next {
        ($bytes: expr) => {
            #[allow(unused_assignments)]
            {
                let data = $bytes;
                $crate::memory::copy_to_pagetable(vmm_table, addr + current, data);
                current += data.len();
            }
        };
    }
    // Raw slices

    copy_next!(&env.len().to_ne_bytes());
    for slice in env {
        copy_next!(slice);
        copy_next!(&[0]);
    }

    copy_next!(&args.len().to_ne_bytes());
    for slice in args {
        copy_next!(slice.as_bytes());
        copy_next!(&[0]);
    }

    // ================= Copying Pointers =======================
    current = env_pointers_off;
    let mut ptr_at = size_of::<usize>();
    for var in env {
        let ptr = addr + raw_slices_section_off + ptr_at;

        copy_next!(&ptr.to_ne_bytes());
        copy_next!(&var.len().to_ne_bytes());

        ptr_at += var.len() + 1;
    }

    current = args_pointers_off;
    let mut ptr_at = size_of::<usize>();
    for arg in args {
        let ptr = addr + raw_args_section_start + ptr_at;

        copy_next!(&ptr.to_ne_bytes());
        copy_next!(&arg.len().to_ne_bytes());

        ptr_at += arg.len() + 1;
    }
    // ===================== End Copying Pointers =======================
    current = abi_structures_off;
    copy_next!(&unsafe {
        core::mem::transmute::<AbiStructures, [u8; size_of::<AbiStructures>()]>(abi_structures)
    });
    // ===================== End Allocation & Copying =======================

    let stack_end_off = stack_off + stack_size;
    Ok((
        TrackedMemoryAllocation::new(Some(vmm), addr, None),
        (
            addr + stack_end_off,
            addr + env_pointers_off,
            addr + args_pointers_off,
            addr + abi_structures_off,
        ),
    ))
}

#[inline(always)]
fn allocate_user_stuff(
    name: &'static &'static str,
    vmm: &VirtualMemoryManager,
    size: usize,
) -> Result<VirtAddr, MapToError> {
    vmm.map_new(
        name,
        None,
        size.to_next_page(),
        VMMMFlags::USER_ACCESSIBLE
            | VMMMFlags::ZEROED
            | VMMMFlags::WRITABLE
            | VMMMFlags::EXECUTABLE,
        vmm::VMMAllocMode::Normal,
    )
    .map_err(|e| match e {
        VMMAllocError::OutOfMemory => MapToError::FrameAllocationFailed,
        e => unreachable!("unreachable error: {e:?}"),
    })
}

/// Allocates a new thread user stack
#[inline(always)]
pub fn allocate_user_stack(
    vmm: Arc<VirtualMemoryManager>,
    size: usize,
) -> Result<(TrackedMemoryAllocation, VirtAddr), MapToError> {
    let addr = allocate_user_stuff(&"thread.stack", &vmm, size)?;
    Ok((
        TrackedMemoryAllocation::new(Some(vmm), addr, None),
        addr + size,
    ))
}

/// Allocates a new thread kernel stack.
pub fn allocate_kernel_stack(
    size: usize,
) -> Result<(TrackedMemoryAllocation, VirtAddr), MapToError> {
    let size = size.to_next_page();

    vmm::with_root(|vmm| {
        let addr = vmm
            .map_new(
                &"thread.ke_stack",
                None,
                size,
                VMMMFlags::ZEROED | VMMMFlags::WRITABLE,
                vmm::VMMAllocMode::Normal,
            )
            .map_err(|e| match e {
                VMMAllocError::OutOfMemory => MapToError::FrameAllocationFailed,
                e => unreachable!("unreachable error: {e:?}"),
            })?;

        Ok((TrackedMemoryAllocation::new(None, addr, None), addr + size))
    })
}

/// Allocates a new thread TLS.
///
/// returns (uthread_addr (set TLS reg to this), actual tls_addr).
pub fn allocate_tls(
    vmm: Arc<VirtualMemoryManager>,
    master_tls_addr_within: VirtAddr,
    tls_alignment: usize,
    size: usize,
    tls_file_size: usize,
) -> Result<(TrackedMemoryAllocation, VirtAddr), MapToError> {
    let alloc_alignment = tls_alignment.max(align_of::<UThreadLocalInfo>());

    #[cfg(target_arch = "x86_64")]
    #[repr(C)]
    struct UThreadLocalInfo {
        uthread_ptr: NonNull<u8>,
        thread_local_storage_ptr: NonNull<u8>,
        thread_local_storage_size: usize,
    }

    #[cfg(target_arch = "aarch64")]
    #[repr(C)]
    struct UThreadLocalInfo {
        thread_local_storage_ptr: NonNull<u8>,
        thread_local_storage_size: usize,
    }

    let tls_aligned_size = size.to_next_multiple_of(alloc_alignment);
    let tls_total_size = size_of::<UThreadLocalInfo>() + tls_aligned_size;
    let tls_alloc_addr = allocate_user_stuff(&"thread.tls", &vmm, tls_total_size)?;

    let (uthread_addr, tls_addr) = {
        cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                (tls_alloc_addr + tls_aligned_size, tls_alloc_addr)
            } else if #[cfg(target_arch = "aarch64")] {
                (tls_alloc_addr, tls_alloc_addr + size_of::<UThreadLocalInfo>())
            } else {
                compile_error!("TLS placement not implemented for the current architecture")
            }
        }
    };

    let uthread_info = {
        cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                UThreadLocalInfo {
                    uthread_ptr: unsafe { NonNull::new_unchecked(uthread_addr.into_ptr()) },
                    thread_local_storage_ptr: unsafe { NonNull::new_unchecked(tls_addr.into_ptr()) },
                    thread_local_storage_size: size,
                }
            } else if #[cfg(target_arch = "aarch64")] {
                UThreadLocalInfo {
                    thread_local_storage_ptr: unsafe { NonNull::new_unchecked(tls_addr.into_ptr()) },
                    thread_local_storage_size: size,
                }
            } else {
                compile_error!("TLS placement not implemented for the current architecture")
            }
        }
    };

    let uthread_bytes: [u8; size_of::<UThreadLocalInfo>()] =
        unsafe { core::mem::transmute(uthread_info) };

    let sync_page_table = vmm.table_inner();
    let mut op = sync_page_table.begin();
    let page_table = unsafe { op.page_table_mut() };
    memory::copy_to_pagetable(page_table, uthread_addr, &uthread_bytes);
    // only copy file size
    memory::pagetable_copy_within(page_table, master_tls_addr_within, tls_addr, tls_file_size);
    drop(op);

    Ok((
        TrackedMemoryAllocation::new(Some(vmm), tls_alloc_addr, None),
        uthread_addr,
    ))
}

/// Maps a region of memory (or a device `interface`) into the virtual memory space given a [`Location`] or chooses its own address, returning a [`TrackedMemoryAllocation`].
pub fn mem_map(
    location: Option<Location>,
    page_count: usize,
    flags: VMMMFlags,
    populate: bool,
    interface: Option<Box<dyn MemMappedInterface>>,
) -> Result<TrackedMemoryAllocation, VMMAllocError> {
    let size_bytes = page_count * PAGE_SIZE;
    let flags = VMMMFlags::USER_ACCESSIBLE | VMMMFlags::ZEROED | flags;

    let addr = with_user_vmm(|vmm| {
        if let Some(interface) = interface.as_ref() {
            vmm.map_direct(
                &"MemMappedDevice",
                location,
                size_bytes,
                flags,
                interface.frames().iter().copied(),
            )
        } else {
            let mode = if populate {
                vmm::VMMAllocMode::Normal
            } else {
                vmm::VMMAllocMode::Lazy
            };
            vmm.map_new(&"MemMap", location, page_count * PAGE_SIZE, flags, mode)
        }
    })?;

    Ok(thread::with_current_ref(|thread| {
        TrackedMemoryAllocation::new(Some(thread.process().vmm.clone()), addr, interface)
    }))
}
