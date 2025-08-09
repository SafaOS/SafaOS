use core::{
    mem::ManuallyDrop,
    num::NonZero,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{
    arch::paging::PageTable,
    memory::{
        AlignTo, AlignToPage, copy_to_userspace, proc_mem_allocator::ProcessMemAllocator,
        userspace_copy_within,
    },
    process::vas::ProcVASA,
    scheduler,
    thread::{self, ArcThread, Tid},
    utils::locks::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{
    memory::{paging::MapToError, proc_mem_allocator::TrackedAllocation},
    utils::types::Name,
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use cfg_if::cfg_if;
use safa_abi::process::{AbiStructures, ProcessStdio};
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

pub const PROCESS_AREA_START_ADDR: VirtAddr = VirtAddr::from(0x00007A0000000000);
pub const PROCESS_AREA_SIZE: usize = 0x50000000000;
pub const PROCESS_AREA_END_ADDR: VirtAddr = PROCESS_AREA_START_ADDR + PROCESS_AREA_SIZE;

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
    allocator: Mutex<ProcessMemAllocator>,
    /// The Virtual address space allocator
    vasa: Mutex<ProcVASA>,

    is_alive: AtomicBool,
    /// The exit information of the Process if it has exited
    exit_info: RwLock<Option<ExitInfo>>,
    /// Whether or not the process's memory was cleaned up, Prevents double clean-up
    cleaned_up: AtomicBool,

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

    #[inline]
    fn allocate_thread_local(&self) -> Result<Option<(VirtAddr, TrackedAllocation)>, MapToError> {
        let master_tls = self.master_tls;

        let mut vasa = self.vasa();

        let page_table = &mut vasa.page_table;
        let mut allocator = self.allocator.lock();

        Self::allocate_thread_local_inner(page_table, &mut *allocator, master_tls)
    }

    #[inline]
    fn allocate_thread_local_inner(
        page_table: &mut PageTable,
        allocator: &mut ProcessMemAllocator,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
    ) -> Result<Option<(VirtAddr, TrackedAllocation)>, MapToError> {
        let Some((master_tls_addr, tls_mem_size, tls_file_size, tls_alignment)) = master_tls else {
            return Ok(None);
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

        let size = size_of::<UThreadLocalInfo>() + tls_mem_size;
        let tracker = allocator.allocate_tracked_guraded(size, tls_alignment, 0)?;

        let allocated_start = tracker.start();

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

        copy_to_userspace(page_table, uthread_addr, &uthread_bytes);
        // only copy file size
        userspace_copy_within(page_table, master_tls_addr, tls_addr, tls_file_size);

        Ok(Some((uthread_addr, tracker)))
    }

    fn allocate_stack_inner(
        allocator: &mut ProcessMemAllocator,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<TrackedAllocation, MapToError> {
        allocator.allocate_tracked_guraded(
            custom_stack_size
                .map(|v| v.get())
                .unwrap_or(DEFAULT_STACK_SIZE)
                .to_next_multiple_of(0x10usize),
            PAGE_SIZE,
            GUARD_PAGES_COUNT,
        )
    }

    fn allocate_stack(
        &self,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<TrackedAllocation, MapToError> {
        Self::allocate_stack_inner(&mut *self.allocator.lock(), custom_stack_size)
    }

    fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        default_priority: ContextPriority,
        root_page_table: PhysPageTable,
        cwd: Box<PathBuf>,
        data_break: VirtAddr,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
        allocator: ProcessMemAllocator,
        userspace_process: bool,
    ) -> Self {
        Self {
            name,
            pid,

            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            cleaned_up: AtomicBool::new(false),
            threads: Mutex::new(Vec::new()),

            next_tid: AtomicU32::new(1),
            master_tls,
            context_count: AtomicU32::new(0),
            default_priority,
            exit_info: RwLock::new(None),
            vasa: Mutex::new(ProcVASA::new(root_page_table, data_break)),
            resources: RwLock::new(ResourceManager::new()),
            cwd: RwLock::new(cwd),
            allocator: Mutex::new(allocator),
            userspace_process,
        }
    }

    /// Creates a new process returning a combination of the process and the main thread
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
    ) -> Result<(Arc<Self>, ArcThread), MapToError> {
        let data_break = data_break.to_next_page();
        let mut root_page_table = root_page_table;

        let mut proc_mem_allocator = ProcessMemAllocator::new(
            &mut *root_page_table,
            PROCESS_AREA_START_ADDR,
            PROCESS_AREA_SIZE,
        );

        let envc = env.len();
        let (_, _, envv_start) = proc_mem_allocator.allocate_filled_with_slices(env, 0x10)?;

        let argc = args.len();
        let (_, _, argv_start) = proc_mem_allocator
            .allocate_filled_with_slices(unsafe { core::mem::transmute(args) }, 0x10)?;

        let structures = AbiStructures::new(stdio, pid, crate::arch::available_cpus());
        let (abi_structures_start, _) = proc_mem_allocator.allocate_filled_with(
            &unsafe { core::mem::transmute::<_, [u8; size_of::<AbiStructures>()]>(structures) }[..],
            align_of::<AbiStructures>(),
        )?;

        let entry_args = [
            argc,
            argv_start.into_raw(),
            envc,
            envv_start.into_raw(),
            abi_structures_start.into_raw(),
        ];

        let mut to_track = heapless::Vec::new();

        let user_stack_tracker =
            Self::allocate_stack_inner(&mut proc_mem_allocator, custom_stack_size)?;
        let kernel_stack_tracker =
            Self::allocate_stack_inner(&mut proc_mem_allocator, custom_stack_size)?;

        let tls = Self::allocate_thread_local_inner(
            &mut root_page_table,
            &mut proc_mem_allocator,
            master_tls,
        )?;

        let (tls_addr, tls_tracker) = match tls {
            Some((tls_addr, tracker)) => (tls_addr, Some(tracker)),
            None => (VirtAddr::null(), None),
        };

        let context = unsafe {
            CPUStatus::create_root(
                &mut root_page_table,
                entry_point,
                entry_args,
                tls_addr,
                user_stack_tracker.end(),
                kernel_stack_tracker.end(),
                userspace_process,
            )?
        };

        to_track.push(user_stack_tracker).unwrap();
        to_track.push(kernel_stack_tracker).unwrap();
        if let Some(tracker) = tls_tracker {
            to_track.push(tracker).unwrap();
        }

        let process = Arc::new(Self::new(
            name,
            pid,
            ppid,
            default_priority,
            root_page_table,
            cwd,
            data_break,
            master_tls,
            proc_mem_allocator,
            userspace_process,
        ));

        let root_thread = ArcThread::new(Self::create_thread(&process, 0, context, None, to_track));
        process.add_thread(root_thread.clone());

        Ok((process, root_thread))
    }

    fn create_thread(
        process: &Arc<Process>,
        tid: Tid,
        cpu_status: CPUStatus,
        priority: Option<ContextPriority>,
        tracked_allocations: heapless::Vec<TrackedAllocation, 3>,
    ) -> Thread {
        Thread::new(
            tid,
            cpu_status,
            process,
            priority.unwrap_or(process.default_priority),
            tracked_allocations,
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
        let user_stack_tracker = process.allocate_stack(custom_stack_size)?;
        let kernel_stack_tracker = process.allocate_stack(custom_stack_size)?;
        let tls = process.allocate_thread_local()?;

        let (tls_addr, tls_tracker) = match tls {
            Some((tls_addr, tracker)) => (tls_addr, Some(tracker)),
            None => (VirtAddr::null(), None),
        };

        let mut vasa = process.vasa();
        let page_table = &mut vasa.page_table;

        let cpu_status = unsafe {
            CPUStatus::create_child(
                tls_addr,
                user_stack_tracker.end(),
                kernel_stack_tracker.end(),
                page_table,
                entry_point,
                context_id,
                argument_ptr.into_ptr::<()>(),
                process.userspace_process,
            )?
        };

        let mut to_track = heapless::Vec::new();
        to_track.push(user_stack_tracker).unwrap();
        to_track.push(kernel_stack_tracker).unwrap();
        if let Some(tracker) = tls_tracker {
            to_track.push(tracker).unwrap();
        }

        let thread = Self::create_thread(process, context_id, cpu_status, priority, to_track);
        let thread = ArcThread::new(thread);

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
        // Drop resources
        self.resources_mut()
            .overwrite_resources(ResourceManager::new());

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

    pub(super) fn cleanup(&self) -> (ProcessInfo, Option<PhysPageTable>) {
        let page_table = if self
            .cleaned_up
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let mut vasa = self.vasa();
            // Safety:
            // it hasn't been cleaned up before so we wouldn't have double free
            // as for use after free this doesn't gurantuee anything
            Some(unsafe { ManuallyDrop::take(&mut vasa.page_table) })
        } else {
            None
        };
        (ProcessInfo::from(self), page_table)
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
