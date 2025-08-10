use core::{
    num::NonZero,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{
    memory::{
        AlignTo, AlignToPage, copy_to_userspace,
        paging::{EntryFlags, Page},
        userspace_copy_within,
    },
    process::{
        resources::ResourceData,
        vas::{ProcVASA, TrackedMemoryMapping},
    },
    scheduler,
    thread::{self, ArcThread, Tid},
    utils::locks::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{memory::paging::MapToError, utils::types::Name};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use cfg_if::cfg_if;
use safa_abi::{
    ffi::{slice::Slice, str::Str},
    process::{AbiStructures, ProcessStdio},
};
use serde::Serialize;
use thread::{ContextPriority, Thread};

use crate::{
    VirtAddr,
    arch::threading::CPUStatus,
    debug,
    memory::paging::{PAGE_SIZE, PhysPageTable},
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::PathBuf,
    },
};

use resources::ResourceManager;

pub mod current;
pub mod resources;
pub mod spawn;
pub mod vas;

/// Process ID, a unique identifier for a process (process)
pub type Pid = u32;

#[derive(Debug, Clone, Copy)]
pub struct ExitInfo {
    exit_code: usize,
    killed_by: Pid,
}

pub const PROCESS_AREA_END_ADDR: VirtAddr = VirtAddr::from(0x00007F0000000000);

const DEFAULT_STACK_SIZE: usize = 8 * PAGE_SIZE;
const GUARD_PAGES_COUNT: usize = 2;

pub struct Process {
    name: Name,
    /// constant
    pid: Pid,
    /// process may change it's parent pid
    ppid: AtomicU32,

    resources: RwLock<ResourceManager>,
    cwd: RwLock<Box<PathBuf>>,
    /// The Virtual address space allocator
    vasa: Mutex<ProcVASA>,

    is_alive: AtomicBool,
    /// The exit information of the Process if it has exited
    exit_info: RwLock<Option<ExitInfo>>,

    userspace_process: bool,

    /// The priortiy of the root thread, that other threads will inherit unless otherwise specified
    default_priority: ContextPriority,
    /// Information about the master TLS if it exits
    master_tls: Option<(VirtAddr, usize, usize, usize)>,
    next_tid: AtomicU32,
    pub(super) threads: Mutex<Vec<ArcThread>>,
    pub context_count: AtomicU32,
}

impl core::fmt::Debug for Process {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("process")
            .field("name", &self.name)
            .field("pid", &self.pid)
            .field("ppid", &self.ppid)
            .field("is_alive", &self.is_alive)
            .finish()
    }
}

unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    pub const fn pid(&self) -> Pid {
        self.pid
    }

    pub fn ppid(&self) -> Pid {
        self.ppid.load(Ordering::Relaxed)
    }

    fn allocate_root_thread_memory_inner(
        vasa: &mut ProcVASA,
        custom_stack_size: Option<NonZero<usize>>,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
        args: &[&str],
        env: &[&[u8]],
        abi_structures: AbiStructures,
    ) -> Result<
        (
            TrackedMemoryMapping,
            VirtAddr,
            Option<VirtAddr>,
            VirtAddr,
            VirtAddr,
            VirtAddr,
            TrackedMemoryMapping,
            VirtAddr,
        ),
        MapToError,
    > {
        let env_bytes: usize = env.iter().map(|x| x.len() + 1).sum();
        let args_bytes: usize = env.iter().map(|x| x.len() + 1).sum();

        let env_size =
            /* envv themselves (aligned) */ ((env.len() + 1) * size_of::<Slice<u8>>())
            + (size_of::<usize>() /* envc */ + env_bytes).to_next_multiple_of(size_of::<Slice<u8>>());
        let env_size = env_size.to_next_multiple_of(0x10usize);

        let args_size =
            /* argv themselves (aligned) */ ((args.len() + 1) * size_of::<Str>())
        + (size_of::<usize>() /* argc */ + args_bytes).to_next_multiple_of(size_of::<Str>());
        let args_size = args_size.to_next_multiple_of(0x10usize);

        let extra_stack_bytes =
            (env_size + args_size + size_of::<AbiStructures>()).to_next_multiple_of(0x10usize);

        let (th_mem_tracker, stack_supposed_end, tp_addr, ke_stack_tracker, ke_stack_end) =
            Self::allocate_thread_memory_inner(
                vasa,
                custom_stack_size,
                master_tls,
                extra_stack_bytes,
            )?;
        let page_table = &mut vasa.page_table;

        let env_start = stack_supposed_end - env_size;
        let args_start = env_start - args_size;
        let abi_structures_start = args_start - size_of::<AbiStructures>();

        let stack_end = abi_structures_start.to_previous_multiple_of(0x10);

        let mut copy_slices = |start: VirtAddr, slices: &[&[u8]]| {
            let mut copied = 0;

            macro_rules! copy_bytes {
                ($bytes: expr) => {{
                    let data = $bytes;
                    crate::memory::copy_to_userspace(page_table, start + copied, data);
                    copied += data.len();
                }};
            }

            copy_bytes!(&slices.len().to_ne_bytes());

            let slices_data_area_start = start + copied;
            for slice in slices {
                copy_bytes!(slice);
                copy_bytes!(&[0]);
            }

            copied = copied.to_next_multiple_of(size_of::<Slice<u8>>());
            let pointers_start = start + copied;
            let mut current_slice_data_ptr = slices_data_area_start;

            for slice in slices {
                let raw_slice_fat = unsafe {
                    Slice::from_raw_parts(current_slice_data_ptr.into_ptr::<u8>(), slice.len())
                };
                let bytes: [u8; size_of::<Slice<u8>>()] =
                    unsafe { core::mem::transmute(raw_slice_fat) };

                copy_bytes!(&bytes);
                current_slice_data_ptr += slice.len() + 1;
            }

            pointers_start
        };

        let env_pointers_start = copy_slices(env_start, env);
        let argv_pointers_start = copy_slices(args_start, unsafe { core::mem::transmute(args) });
        crate::memory::copy_to_userspace(page_table, abi_structures_start, &unsafe {
            core::mem::transmute::<_, [u8; size_of::<AbiStructures>()]>(abi_structures)
        });
        Ok((
            th_mem_tracker,
            stack_end,
            tp_addr,
            env_pointers_start,
            argv_pointers_start,
            abi_structures_start,
            ke_stack_tracker,
            ke_stack_end,
        ))
    }

    /// Allocates the thread stack, thread local area, and the kernel thread stack, the kernel thread stack will have `extra_stack_bytes` extra bytes
    /// # Returns
    /// (the kernel thread stack and thread local copy tracker, the thread stack end, the TP, the kernel thread stack tracker)
    fn allocate_thread_memory_inner(
        vasa: &mut ProcVASA,
        custom_stack_size: Option<NonZero<usize>>,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
        extra_stack_bytes: usize,
    ) -> Result<
        (
            TrackedMemoryMapping,
            VirtAddr,
            Option<VirtAddr>,
            TrackedMemoryMapping,
            VirtAddr,
        ),
        MapToError,
    > {
        let flags = EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE;
        let stack_size = custom_stack_size
            .map(|v| v.get())
            .unwrap_or(DEFAULT_STACK_SIZE)
            .to_next_page();

        let thread_ke_stack_mapping = vasa.map_n_pages_tracked(
            None,
            stack_size / PAGE_SIZE,
            GUARD_PAGES_COUNT,
            flags,
            core::iter::empty(),
            None,
        )?;

        let ke_stack_end = thread_ke_stack_mapping.end();

        let size = stack_size
            + if let Some((_, tls_mem_size, _, tls_alignment)) = master_tls {
                (tls_mem_size + size_of::<UThreadLocalInfo>())
                    .to_next_multiple_of(tls_alignment)
                    .to_next_multiple_of(0x10usize)
            } else {
                0
            }
            + extra_stack_bytes;
        let size = size.to_next_page();

        let thread_space_mapping = vasa.map_n_pages_tracked(
            None,
            size / PAGE_SIZE,
            GUARD_PAGES_COUNT,
            flags,
            core::iter::empty(),
            None,
        )?;

        let mapping_end = thread_space_mapping.end();
        let Some((master_tls_addr, tls_mem_size, tls_file_size, tls_alignment)) = master_tls else {
            return Ok((
                thread_space_mapping,
                mapping_end,
                None,
                thread_ke_stack_mapping,
                ke_stack_end,
            ));
        };
        assert!(tls_alignment >= align_of::<UThreadLocalInfo>());

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

        let tls_v_size =
            (size_of::<UThreadLocalInfo>() + tls_mem_size).to_next_multiple_of(tls_alignment);
        let allocated_start = mapping_end - tls_v_size;
        let stack_end = allocated_start.to_previous_multiple_of(0x10);

        let (uthread_addr, tls_addr) = {
            cfg_if! {
                if #[cfg(target_arch = "x86_64")] {
                    (allocated_start + tls_mem_size, allocated_start)
                } else if #[cfg(target_arch = "aarch64")] {
                    (allocated_start, allocated_start + size_of::<UThreadLocalInfo>())
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
                        thread_local_storage_size: tls_mem_size,
                    }
                } else if #[cfg(target_arch = "aarch64")] {
                    UThreadLocalInfo {
                        thread_local_storage_ptr: unsafe { NonNull::new_unchecked(tls_addr.into_ptr()) },
                        thread_local_storage_size: tls_mem_size,
                    }
                } else {
                    compile_error!("TLS placement not implemented for the current architecture")
                }
            }
        };

        let uthread_bytes: [u8; size_of::<UThreadLocalInfo>()] =
            unsafe { core::mem::transmute(uthread_info) };

        let page_table = &mut vasa.page_table;
        copy_to_userspace(page_table, uthread_addr, &uthread_bytes);
        // only copy file size
        userspace_copy_within(page_table, master_tls_addr, tls_addr, tls_file_size);

        Ok((
            thread_space_mapping,
            stack_end,
            Some(uthread_addr),
            thread_ke_stack_mapping,
            ke_stack_end,
        ))
    }

    fn allocate_thread_memory(
        &self,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<
        (
            TrackedMemoryMapping,
            VirtAddr,
            Option<VirtAddr>,
            TrackedMemoryMapping,
            VirtAddr,
        ),
        MapToError,
    > {
        Self::allocate_thread_memory_inner(&mut *self.vasa(), custom_stack_size, self.master_tls, 0)
    }

    fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        default_priority: ContextPriority,
        cwd: Box<PathBuf>,
        vasa: ProcVASA,
        resources: ResourceManager,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
        userspace_process: bool,
    ) -> Self {
        Self {
            name,
            pid,

            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            threads: Mutex::new(Vec::new()),

            next_tid: AtomicU32::new(1),
            master_tls,
            context_count: AtomicU32::new(0),
            default_priority,
            exit_info: RwLock::new(None),
            vasa: Mutex::new(vasa),
            resources: RwLock::new(resources),
            cwd: RwLock::new(cwd),
            userspace_process,
        }
    }

    /// Creates a new process returning a combination of the process, the main thread, and resources that should be added to the process
    pub fn create(
        name: Name,
        pid: Pid,
        ppid: Pid,
        entry_point: VirtAddr,
        cwd: Box<PathBuf>,
        env: &[&[u8]],
        args: &[&str],
        stdio: ProcessStdio,
        root_page_table: PhysPageTable,
        data_break: VirtAddr,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
        default_priority: ContextPriority,
        userspace_process: bool,
        custom_stack_size: Option<NonZero<usize>>,
        with_resources: Option<ResourceManager>,
    ) -> Result<(Arc<Self>, ArcThread), MapToError> {
        let data_break = data_break.to_next_page();
        let mut vasa = ProcVASA::new(root_page_table, data_break);
        let mut resources = with_resources.unwrap_or(ResourceManager::new());
        let abi_structures = AbiStructures::new(stdio, pid, crate::arch::available_cpus());

        let (
            thread_mem_tracker,
            stack_end,
            tp_addr,
            envv_pointers_start,
            argv_pointers_start,
            abi_structers_start,
            ke_stack_tracker,
            ke_stack_end,
        ) = Self::allocate_root_thread_memory_inner(
            &mut vasa,
            custom_stack_size,
            master_tls,
            args,
            env,
            abi_structures,
        )?;
        let entry_args = [
            args.len(),
            argv_pointers_start.into_raw(),
            env.len(),
            envv_pointers_start.into_raw(),
            abi_structers_start.into_raw(),
        ];

        let context = unsafe {
            let root_page_table = &mut vasa.page_table;
            assert!(
                root_page_table
                    .get_frame(Page::containing_address(stack_end))
                    .is_some()
            );
            CPUStatus::create_root(
                root_page_table,
                entry_point,
                entry_args,
                tp_addr.unwrap_or(VirtAddr::null()),
                stack_end,
                ke_stack_end,
                userspace_process,
            )?
        };

        resources.add_global_resource(ResourceData::TrackedMapping(Arc::new(thread_mem_tracker)));
        resources.add_global_resource(ResourceData::TrackedMapping(Arc::new(ke_stack_tracker)));

        let process = Arc::new(Self::new(
            name,
            pid,
            ppid,
            default_priority,
            cwd,
            vasa,
            resources,
            master_tls,
            userspace_process,
        ));

        let root_thread = ArcThread::new(Self::create_thread(&process, 0, context, None));
        process.add_thread(root_thread.clone());

        Ok((process, root_thread))
    }

    fn create_thread(
        process: &Arc<Process>,
        tid: Tid,
        cpu_status: CPUStatus,
        priority: Option<ContextPriority>,
    ) -> Thread {
        Thread::new(
            tid,
            cpu_status,
            process,
            priority.unwrap_or(process.default_priority),
        )
    }

    fn add_thread(&self, thread: ArcThread) {
        self.threads.lock().push(thread);
        self.context_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Creates a new thread from a CPU status giving it a `cid` and everything
    /// adds to the process's context count so it tracks this thread
    pub fn new_thread(
        process: &Arc<Process>,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        priority: Option<ContextPriority>,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<(ArcThread, Tid), MapToError> {
        let context_id = process.next_tid.fetch_add(1, Ordering::SeqCst);

        let (th_mem_tracker, stack_end, tp_addr, ke_stack_tracker, ke_stack_end) =
            process.allocate_thread_memory(custom_stack_size)?;

        let mut vasa = process.vasa();
        let page_table = &mut vasa.page_table;

        let cpu_status = unsafe {
            CPUStatus::create_child(
                tp_addr.unwrap_or(VirtAddr::null()),
                stack_end,
                ke_stack_end,
                page_table,
                entry_point,
                context_id,
                argument_ptr.into_ptr::<()>(),
                process.userspace_process,
            )?
        };

        let thread = Self::create_thread(process, context_id, cpu_status, priority);
        let thread = ArcThread::new(thread);

        let mut resources = process.resources_mut();
        let th_mem_ri =
            resources.add_local_resource(ResourceData::TrackedMapping(Arc::new(th_mem_tracker)));
        let ke_stack_ri =
            resources.add_local_resource(ResourceData::TrackedMapping(Arc::new(ke_stack_tracker)));

        thread.take_resources(&[th_mem_ri, ke_stack_ri]);
        process.add_thread(thread.clone());

        Ok((thread, context_id))
    }

    /// Creates a new process from an elf
    /// that process is assumed to be in the userspace
    pub fn from_elf<T: Readable>(
        name: Name,
        pid: Pid,
        ppid: Pid,
        cwd: Box<PathBuf>,
        elf: Elf<T>,
        args: &[&str],
        env: &[&[u8]],
        default_priority: ContextPriority,
        stdio: ProcessStdio,
        custom_stack_size: Option<NonZero<usize>>,
        with_resources: Option<ResourceManager>,
    ) -> Result<(Arc<Self>, ArcThread), ElfError> {
        let entry_point = elf.header().entry_point;
        let mut page_table = PhysPageTable::create()?;
        let (data_break, master_tls) = elf.load_exec(&mut page_table)?;

        Self::create(
            name,
            pid,
            ppid,
            entry_point,
            cwd,
            env,
            args,
            stdio,
            page_table,
            data_break,
            master_tls,
            default_priority,
            true,
            custom_stack_size,
            with_resources,
        )
        .map_err(|e| e.into())
    }

    pub fn name(&self) -> &Name {
        &self.name
    }

    pub fn cwd<'s>(&'s self) -> RwLockReadGuard<'s, Box<PathBuf>> {
        self.cwd.read()
    }

    pub fn cwd_mut<'s>(&'s self) -> RwLockWriteGuard<'s, Box<PathBuf>> {
        self.cwd.write()
    }

    pub fn resources<'s>(&'s self) -> RwLockReadGuard<'s, ResourceManager> {
        self.resources.read()
    }

    pub fn resources_mut<'s>(&'s self) -> RwLockWriteGuard<'s, ResourceManager> {
        self.resources.write()
    }

    fn vasa<'s>(&'s self) -> MutexGuard<'s, ProcVASA> {
        self.vasa.lock()
    }

    /// kills the process
    /// if `killed_by` is `None` the process will be killed by itself
    /// # Safety
    /// If this function was called on the current process, the caller must call it without interrupts enabled.
    pub unsafe fn kill(&self, exit_code: usize, killed_by: Option<Pid>) {
        let pid = self.pid();
        let killed_by = killed_by.unwrap_or(pid);

        let threads = self.threads.lock();
        // Set state to dead
        *self.exit_info.write() = Some(ExitInfo {
            exit_code,
            killed_by,
        });

        for thread in &*threads {
            if !thread.is_dead() {
                unsafe { thread.soft_kill(true) };
            }
        }

        debug!(
            Process,
            "Process {} ({}) TERMINATED with code {} by {}",
            pid,
            self.name(),
            exit_code,
            killed_by
        );

        drop(threads);
        self.is_alive.store(false, Ordering::Release);
    }

    pub(super) fn info(&self) -> ProcessInfo {
        ProcessInfo::from(self)
    }

    /// Attempts to wake up `n` threads waiting on the futex at `target_addr`.
    /// Returns the number of threads that were successfully woken up.
    pub(super) fn wake_n_futexs(&self, target_addr: *const AtomicU32, n: usize) -> usize {
        if n == 0 {
            return 0;
        }

        let mut count = 0;

        for thread in &*self.threads.lock() {
            let mut status = thread.status_mut();
            if status.try_lift_futex(target_addr) {
                count += 1;
                if count >= n {
                    break;
                }
            }
        }

        return count;
    }

    fn at(&self) -> VirtAddr {
        VirtAddr::null()
    }

    fn stack_at(&self) -> VirtAddr {
        VirtAddr::null()
    }

    pub(super) fn is_alive(&self) -> bool {
        self.is_alive.load(core::sync::atomic::Ordering::Acquire)
    }
}

/// Returns the current process. (The process that is a parent of the current thread)
pub fn current() -> Arc<Process> {
    thread::current().process().clone()
}

/// Fast, cheaper access to the current process's pid
pub fn current_pid() -> Pid {
    thread::current_pid()
}

#[derive(Serialize, Debug, Clone)]
#[repr(C)]
pub struct ProcessInfo {
    name: Name,

    pub ppid: Pid,
    pub pid: Pid,

    pub at: VirtAddr,
    pub stack_addr: VirtAddr,

    pub killed_by: Option<Pid>,
    pub exit_code: Option<usize>,
    pub is_alive: bool,
}

impl From<&Process> for ProcessInfo {
    fn from(process: &Process) -> Self {
        let at = process.at();
        let stack_addr = process.stack_at();

        let exit_info = process.exit_info.read();
        let (exit_code, killed_by) = match &*exit_info {
            Some(i) => (Some(i.exit_code), Some(i.killed_by)),
            None => (None, None),
        };

        let is_alive = process.is_alive();
        let ppid = process.ppid.load(core::sync::atomic::Ordering::Relaxed);
        let name = process.name().clone();

        Self {
            ppid,
            pid: process.pid(),
            name,
            exit_code,
            at,
            stack_addr,

            killed_by,
            is_alive,
        }
    }
}

/// Returns [`ProcessInfo`] for the process with the given PID.
pub fn getinfo(pid: Pid) -> Option<ProcessInfo> {
    scheduler::process_list::find(|p| p.pid() == pid, |t| ProcessInfo::from(&**t))
}
