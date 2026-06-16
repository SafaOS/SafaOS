use core::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    num::NonZero,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{
    memory::{frame_allocator::SIZE_1M, vmm::VirtualMemoryManager},
    process::threads::ThreadsManager,
    scheduler::{
        self,
        wait_queue::{WaitError, WaitQueue, WaitQueueWithTimeout},
    },
    thread::{self, ArcThread},
    utils::locks::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{memory::paging::MapToError, utils::types::Name};
use alloc::{boxed::Box, sync::Arc};
use safa_abi::process::{AbiStructures, ProcessStdio};
use serde::Serialize;
use thread::ContextPriority;

use crate::{
    VirtAddr, debug,
    memory::paging::PhysPageTable,
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::PathBuf,
    },
};

use resources::ResourceManager;

pub mod current;
pub mod mem;
pub mod poll;
pub mod resources;
pub mod spawn;
pub mod threads;

// pub mod vas;

/// Process ID, a unique identifier for a process (process)
pub type Pid = u32;

#[derive(Debug, Clone, Copy)]
pub struct ExitInfo {
    exit_code: isize,
    killed_by: Pid,
}

pub const PROCESS_AREA_END_ADDR: VirtAddr = VirtAddr::from(0x00007F0000000000);

const DEFAULT_STACK_SIZE: usize = SIZE_1M;

/// Reason for waiting inside a process's wait queue.
#[derive(Debug, Clone)]
pub enum WaitOnProcReason {
    WaitingOnSelf,
    WaitingOnChild(ArcThread),
    WaitingOnFutex(*const AtomicU32, u32),
}

pub struct Process {
    name: Name,
    /// constant
    pid: Pid,
    /// process may change it's parent pid
    ppid: AtomicU32,

    resources: RwLock<ResourceManager>,
    cwd: RwLock<Box<PathBuf>>,
    pub(super) vmm: Arc<VirtualMemoryManager>,
    page_table: UnsafeCell<ManuallyDrop<PhysPageTable>>,
    is_alive: AtomicBool,
    /// The exit information of the Process if it has exited
    exit_info: RwLock<Option<ExitInfo>>,

    /// The priortiy of the root thread, that other threads will inherit unless otherwise specified
    default_priority: ContextPriority,
    threads_manager: Mutex<ThreadsManager>,
    is_dying: AtomicBool,
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
            let mut resources = self
                .resources
                .try_write()
                .expect("Process should be inactive");

            // Safety: We know the process is dead if this function was called.
            resources.drop_all();
            ManuallyDrop::drop(&mut self.page_table.as_mut_unchecked());
        }
    }

    /// Called when a thread exits.
    /// # Safety
    /// This function is unsafe because it can be called from any thread, and it can cause the process to exit, also it requires interrupts to be disabled if from the current process.
    pub unsafe fn on_thread_exit(
        this: &Arc<Process>,
        thread: &ArcThread,
        exit_code: isize,
    ) -> bool {
        let tid = thread.tid();

        unsafe {
            let success = thread.soft_kill(false);
            if success {
                let process_dead = this
                    .context_count
                    .fetch_sub(1, core::sync::atomic::Ordering::SeqCst)
                    <= 1;

                let mut wait_queue = this.wait_queue.lock();
                wait_queue.wake_on_condition(|r| match r {
                    WaitOnProcReason::WaitingOnChild(child) => child.tid() == tid,
                    _ => false,
                });

                if process_dead {
                    Process::finalize_kill(this, exit_code, this.pid(), &mut *wait_queue)
                };

                return process_dead;
            }

            false
        }
    }

    const fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        default_priority: ContextPriority,
        cwd: Box<PathBuf>,
        vmm: Arc<VirtualMemoryManager>,
        page_table: PhysPageTable,
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
            vmm,
            page_table: UnsafeCell::new(ManuallyDrop::new(page_table)),
            resources: RwLock::new(resources),
            cwd: RwLock::new(cwd),
            is_dying: AtomicBool::new(false),
        }
    }

    /// Creates a new process returning a combination of the process, and the main thread.
    pub fn create(
        name: Name,
        pid: Pid,
        ppid: Pid,
        entry_point: VirtAddr,
        cwd: Box<PathBuf>,
        env: &[&[u8]],
        args: &[&str],
        stdio: ProcessStdio,
        vmm: Arc<VirtualMemoryManager>,
        root_page_table: PhysPageTable,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
        default_priority: ContextPriority,
        userspace_process: bool,
        custom_stack_size: Option<NonZero<usize>>,
        with_resources: Option<ResourceManager>,
    ) -> Result<(Arc<Self>, ArcThread), MapToError> {
        let resources = with_resources.unwrap_or(ResourceManager::new());
        let abi_structures = AbiStructures::new(stdio, pid, crate::arch::available_cpus());

        let mut process = Arc::new(Self::new(
            name,
            pid,
            ppid,
            default_priority,
            cwd,
            vmm,
            root_page_table,
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
        let page_table = PhysPageTable::create()?;
        let mut vmm = VirtualMemoryManager::new_user(page_table.frame_ptr());
        let (_, master_tls) = elf.load_exec(&mut vmm)?;

        Self::create(
            name,
            pid,
            ppid,
            entry_point,
            cwd,
            env,
            args,
            stdio,
            Arc::new(vmm),
            page_table,
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

    /// Attempts to acquire the threads manager lock, returning an error if the process is dying.
    pub fn threads_manager<'s>(&'s self) -> MutexGuard<'s, ThreadsManager> {
        self.threads_manager.lock()
    }

    pub fn try_threads_manager<'s>(&'s self) -> Option<MutexGuard<'s, ThreadsManager>> {
        self.threads_manager.try_lock()
    }

    /// Called after the last thread in a process is killed successfully.
    unsafe fn finalize_kill(
        this: &Arc<Process>,
        exit_code: isize,
        killed_by: Pid,
        wait_queue: &mut WaitQueueWithTimeout<3, WaitOnProcReason>,
    ) {
        *this.exit_info.write() = Some(ExitInfo {
            exit_code,
            killed_by,
        });

        this.is_alive.store(false, Ordering::Release);
        wait_queue.wake_all();

        debug!(
            Process,
            "Process {} ({}) TERMINATED with code {} by {}",
            this.pid(),
            this.name(),
            exit_code,
            killed_by
        );
    }

    // TODO: Implement ArcProcess
    /// kills the process
    /// if `killed_by` is `None` the process will be killed by itself
    /// # Safety
    /// If this function was called on the current process, the caller must call it without interrupts enabled.
    pub unsafe fn kill(this: &Arc<Process>, exit_code: isize, killed_by: Option<Pid>) {
        let pid = this.pid();
        let killed_by = killed_by.unwrap_or(pid);

        this.is_dying.store(true, Ordering::Release);
        // We cannot attempt to acquire any locks after this point:
        let mut threads = this.threads_manager.lock();
        let mut wait_queue = this.wait_queue.lock();

        unsafe { threads.kill_all() };
        unsafe { Self::finalize_kill(&this, exit_code, killed_by, &mut *wait_queue) }
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
                WaitOnProcReason::WaitingOnFutex(addr, _) => *addr == target_addr,
                _ => false,
            },
            n,
        );

        return count;
    }

    /// Sleeps the current thread in the process's wait queue.
    pub fn sleep_thread(
        &self,
        reason: WaitOnProcReason,
        duration: Option<NonZero<u64>>,
    ) -> Result<(), WaitError> {
        let pending = self.wait_queue.prepare_wait();
        let cont = match &reason {
            WaitOnProcReason::WaitingOnChild(child) => !child.is_dead(),
            WaitOnProcReason::WaitingOnSelf => self.is_alive(),
            WaitOnProcReason::WaitingOnFutex(addr, value) => unsafe {
                (**addr).load(Ordering::SeqCst) == *value
            },
        };
        if !cont {
            return Ok(());
        }

        pending.enter_wait(reason, duration)
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
    thread::with_current_ref(|curr| curr.process().clone())
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
    pub exit_code: Option<isize>,
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
