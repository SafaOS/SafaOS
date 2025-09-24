use core::{
    mem::ManuallyDrop,
    num::NonZero,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{
    eve,
    memory::{AlignTo, AlignToPage, copy_to_userspace, paging::EntryFlags, userspace_copy_within},
    process::{
        threads::ThreadsManager,
        vas::{ProcVASA, TrackedMemoryMapping},
    },
    scheduler::{
        self,
        wait_queue::{WaitQueue, WaitQueueWithTimeout},
    },
    thread::{self, ArcThread},
    utils::locks::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{memory::paging::MapToError, utils::types::Name};
use alloc::{boxed::Box, sync::Arc};
use cfg_if::cfg_if;
use safa_abi::{
    ffi::{slice::Slice, str::Str},
    process::{AbiStructures, ProcessStdio},
};
use serde::Serialize;
use thread::ContextPriority;

use crate::{
    VirtAddr, debug,
    memory::paging::{PAGE_SIZE, PhysPageTable},
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::PathBuf,
    },
};

use resources::ResourceManager;

pub mod current;
pub mod poll;
pub mod resources;
pub mod spawn;
pub mod threads;
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

/// Reason for waiting inside a process's wait queue.
#[derive(Debug, Clone)]
pub enum WaitOnProcReason {
    WaitingOnSelf,
    WaitingOnChild(ArcThread),
    WaitingOnFutex(*const AtomicU32),
}

pub struct Process {
    name: Name,
    /// constant
    pid: Pid,
    /// process may change it's parent pid
    ppid: AtomicU32,

    resources: RwLock<ResourceManager>,
    cwd: RwLock<Box<PathBuf>>,
    /// The Virtual address space allocator
    pub(super) vasa: Mutex<ProcVASA>,

    is_alive: AtomicBool,
    /// The exit information of the Process if it has exited
    exit_info: RwLock<Option<ExitInfo>>,

    /// The priortiy of the root thread, that other threads will inherit unless otherwise specified
    default_priority: ContextPriority,
    threads_manager: Mutex<ThreadsManager>,
    wait_queue: Mutex<WaitQueueWithTimeout<3, WaitOnProcReason>>,
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

    /// Cleans up the Resources and the memory space of self
    /// # Safety
    ///
    /// All threads must first be removed, switched from and cleaned-up.
    unsafe fn cleanup(&self) {
        unsafe {
            assert!(!self.vasa.is_locked());
            assert!(!self.resources.is_locked());

            *self.resources.write() = ResourceManager::new();
            ManuallyDrop::drop(&mut self.vasa.lock().page_table);
        }
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
        for v in env {
            crate::serial!("env: ");
            for ch in v.utf8_chunks() {
                crate::serial!("{}", ch.valid());
                crate::serial!("{:?}", ch.invalid());
            }
            crate::serial!("\n");
        }
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
    pub(super) fn allocate_thread_memory_inner(
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

        let thread_ke_stack_mapping =
            vasa.map_n_pages_tracked(None, stack_size / PAGE_SIZE, GUARD_PAGES_COUNT, flags)?;

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

        let thread_space_mapping =
            vasa.map_n_pages_tracked(None, size / PAGE_SIZE, GUARD_PAGES_COUNT, flags)?;

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

    /// Called when a thread exits.
    /// # Safety
    /// This function is unsafe because it can be called from any thread, and it can cause the process to exit, also it requires interrupts to be disabled if from the current process.
    pub unsafe fn on_thread_exit(
        this: &Arc<Process>,
        thread: &ArcThread,
        exit_code: usize,
    ) -> bool {
        let tid = thread.tid();

        let process_dead = this
            .context_count
            .fetch_sub(1, core::sync::atomic::Ordering::SeqCst)
            <= 1;

        unsafe {
            thread.soft_kill(process_dead);
            this.wait_queue.lock().wake_on_condition(|r| match r {
                WaitOnProcReason::WaitingOnChild(child) => child.tid() == tid,
                _ => false,
            });
        }
        if process_dead {
            unsafe { Process::kill(this, exit_code, None) };
        }

        process_dead
    }

    const fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        default_priority: ContextPriority,
        cwd: Box<PathBuf>,
        vasa: ProcVASA,
        resources: ResourceManager,
    ) -> Self {
        Self {
            name,
            pid,

            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            threads_manager: Mutex::new(ThreadsManager::new_uninit()),
            wait_queue: Mutex::new(WaitQueue::new()),
            context_count: AtomicU32::new(0),
            default_priority,
            exit_info: RwLock::new(None),
            vasa: Mutex::new(vasa),
            resources: RwLock::new(resources),
            cwd: RwLock::new(cwd),
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
        let vasa = ProcVASA::new(root_page_table, data_break);
        let resources = with_resources.unwrap_or(ResourceManager::new());
        let abi_structures = AbiStructures::new(stdio, pid, crate::arch::available_cpus());

        let mut process = Arc::new(Self::new(
            name,
            pid,
            ppid,
            default_priority,
            cwd,
            vasa,
            resources,
        ));

        unsafe {
            let (manager, thread) = ThreadsManager::new_with_root_thread(
                &mut process,
                entry_point,
                custom_stack_size,
                args,
                env,
                abi_structures,
                userspace_process,
                master_tls,
            )?;

            *process.threads_manager.get() = manager;
            Ok((process, thread))
        }
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

    pub fn vasa<'s>(&'s self) -> MutexGuard<'s, ProcVASA> {
        self.vasa.lock()
    }

    pub fn threads_manager<'s>(&'s self) -> MutexGuard<'s, ThreadsManager> {
        self.threads_manager.lock()
    }

    fn can_cleanup_proc(&self) -> bool {
        self.threads_manager
            .try_lock()
            .is_some_and(|guard| guard.is_empty())
    }

    /// Attempts to cleanup process if all it's thread were already removed, the memory space can be deallocated as a whole.
    /// returns true if the process was cleaned up, false otherwise
    pub fn try_cleanup(&self) -> bool {
        if self.can_cleanup_proc() {
            unsafe {
                self.cleanup();
            }

            true
        } else {
            false
        }
    }

    // TODO: Implement ArcProcess
    /// kills the process
    /// if `killed_by` is `None` the process will be killed by itself
    /// # Safety
    /// If this function was called on the current process, the caller must call it without interrupts enabled.
    pub unsafe fn kill(this: &Arc<Process>, exit_code: usize, killed_by: Option<Pid>) {
        let pid = this.pid();
        let killed_by = killed_by.unwrap_or(pid);

        // !!!!! Cleanup must be done before this thread is removed !!!!!
        eve::schedule_proc_cleanup(this.clone());

        let mut threads = this.threads_manager.lock();
        // Set state to dead
        *this.exit_info.write() = Some(ExitInfo {
            exit_code,
            killed_by,
        });

        threads.kill_all();
        this.is_alive.store(false, Ordering::Release);
        this.wait_queue.lock().wake_all();

        debug!(
            Process,
            "Process {} ({}) TERMINATED with code {} by {}",
            pid,
            this.name(),
            exit_code,
            killed_by
        );
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

        let count = self.wait_queue.lock().wake_n_on_condition(
            |reason| match reason {
                WaitOnProcReason::WaitingOnFutex(addr) => *addr == target_addr,
                _ => false,
            },
            n,
        );

        return count;
    }

    /// Sleeps the current thread in the process's wait queue.
    pub fn sleep_thread(
        &self,
        thread: ArcThread,
        reason: WaitOnProcReason,
        duration: Option<NonZero<u64>>,
    ) -> Option<NonZero<u64>> {
        let mut queue = self.wait_queue.lock();
        if let WaitOnProcReason::WaitingOnChild(ref child) = reason {
            if child.is_dead() {
                return None;
            }
        }

        queue.push(thread, reason, duration)
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
