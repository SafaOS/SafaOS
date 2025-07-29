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
    scheduler,
    thread::{self, Tid},
    utils::locks::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard},
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
    memory::paging::{PAGE_SIZE, Page, PhysPageTable},
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::{Path, PathBuf},
    },
};

use crate::scheduler::resources::{Resource, ResourceManager};

pub mod current;
pub mod spawn;

/// Process ID, a unique identifier for a process (process)
pub type Pid = u32;

#[derive(Debug)]
pub struct AliveProcess {
    root_page_table: ManuallyDrop<PhysPageTable>,
    resources: ResourceManager,

    data_pages: usize,
    data_start: VirtAddr,
    data_break: VirtAddr,
    master_tls: Option<(VirtAddr, usize, usize)>,
    cwd: Box<PathBuf>,
}
#[derive(Debug)]
pub struct ZombieProcess {
    exit_code: usize,
    killed_by: Pid,

    data_start: VirtAddr,
    data_break: VirtAddr,

    last_resource_id: usize,
    cwd: Box<PathBuf>,
    root_page_table: ManuallyDrop<PhysPageTable>,
}

impl AliveProcess {
    pub fn resource_manager(&self) -> &ResourceManager {
        &self.resources
    }

    pub fn resource_manager_mut(&mut self) -> &mut ResourceManager {
        &mut self.resources
    }
    pub fn cwd<'s>(&'s self) -> Path<'s> {
        self.cwd.as_path()
    }

    /// Clones the resources of `self`, panicks if self isn't alive
    pub fn clone_resources(&mut self) -> Vec<Mutex<Resource>> {
        self.resources.clone_resources()
    }

    /// Clones only the resources in `resources` of `self`
    ///
    /// # Returns
    /// A vector of cloned resources, or an error if any resource fails to clone because it doesn't exist
    pub fn clone_specific_resources(
        &mut self,
        resources: &[usize],
    ) -> Result<Vec<Mutex<Resource>>, ()> {
        if resources.is_empty() {
            return Ok(Vec::new());
        }

        let biggest = resources.iter().max().copied().unwrap_or(0);
        // ensures the results has the same ids as the resources
        let mut results = Vec::with_capacity(biggest + 1);
        results.resize_with(biggest + 1, || Mutex::new(Resource::Null));

        for resource in resources {
            let result = self.resources.clone_resource(*resource).ok_or(())?;
            results[*resource] = result;
        }

        Ok(results)
    }

    pub fn cwd_mut(&mut self) -> &mut PathBuf {
        &mut self.cwd
    }

    fn page_extend_data(&mut self) -> Option<VirtAddr> {
        use crate::memory::paging::EntryFlags;

        let page_end = self.data_start + PAGE_SIZE * self.data_pages;
        let new_page = Page::containing_address(page_end);

        unsafe {
            if let Err(e) = self
                .root_page_table
                .map_zeroed(new_page, EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE)
            {
                match e {
                    MapToError::FrameAllocationFailed => {
                        return None;
                    }
                    MapToError::AlreadyMapped => {
                        panic!("attempted to extend data break beyond an already mapped territory")
                    }
                }
            }
        }

        self.data_pages += 1;
        Some(new_page.virt_addr())
    }

    fn page_unextend_data(&mut self) -> Option<VirtAddr> {
        if self.data_pages == 0 {
            return Some(self.data_start);
        }

        let page_end = self.data_start + PAGE_SIZE * self.data_pages;
        let page_addr = page_end - PAGE_SIZE;
        let page = Page::containing_address(page_addr);

        unsafe {
            self.root_page_table.unmap(page);
        }

        self.data_pages -= 1;
        Some(page_addr)
    }

    pub fn extend_data_by(&mut self, amount: isize) -> Option<*mut u8> {
        let actual_data_break = self.data_start + PAGE_SIZE * self.data_pages;
        let usable_bytes = actual_data_break - self.data_break;
        let is_negative = amount.is_negative();
        let amount = amount.unsigned_abs();

        if (usable_bytes < amount) || (is_negative) {
            let pages = (amount - usable_bytes).to_next_page() / PAGE_SIZE;

            // FIXME: not tested
            let func = if is_negative {
                Self::page_unextend_data
            } else {
                Self::page_extend_data
            };

            for _ in 0..pages {
                func(self)?;
            }
        }

        if is_negative && amount >= usable_bytes {
            self.data_break -= amount;
        } else {
            self.data_break += amount;
        }

        Some(self.data_break.into_ptr::<u8>())
    }

    /// Makes `self` a zombie
    /// # Safety
    ///  unsafe because `self` becomes invalid after this call
    unsafe fn die_mut(&mut self, exit_code: usize, killed_by: Pid) -> ZombieProcess {
        unsafe {
            ZombieProcess {
                root_page_table: ManuallyDrop::new(ManuallyDrop::take(&mut self.root_page_table)),
                exit_code,
                killed_by,
                data_start: self.data_start,
                data_break: self.data_break,
                last_resource_id: self.resources.next_ri(),
                cwd: core::mem::take(&mut self.cwd),
            }
        }
    }
}

impl ZombieProcess {
    pub fn cwd<'s>(&'s self) -> Path<'s> {
        self.cwd.as_path()
    }
}
#[derive(Debug)]
pub enum ProcessState {
    Alive(AliveProcess),
    Zombie(ZombieProcess),
}

impl ProcessState {
    fn zombie_mut(&mut self) -> Option<&mut ZombieProcess> {
        match self {
            ProcessState::Zombie(zombie) => Some(zombie),
            ProcessState::Alive { .. } => None,
        }
    }

    fn alive(&self) -> Option<&AliveProcess> {
        match self {
            ProcessState::Alive(alive) => Some(alive),
            ProcessState::Zombie { .. } => None,
        }
    }

    fn alive_mut(&mut self) -> Option<&mut AliveProcess> {
        match self {
            ProcessState::Alive(alive) => Some(alive),
            ProcessState::Zombie { .. } => None,
        }
    }

    pub fn resource_manager(&self) -> Option<&ResourceManager> {
        self.alive().map(|alive| alive.resource_manager())
    }

    pub fn resource_manager_mut(&mut self) -> Option<&mut ResourceManager> {
        self.alive_mut().map(|alive| alive.resource_manager_mut())
    }

    pub fn cwd<'s>(&'s self) -> Path<'s> {
        match self {
            ProcessState::Alive(alive) => alive.cwd(),
            ProcessState::Zombie(zombie) => zombie.cwd(),
        }
    }

    /// Clones the resources of `self`, panicks if self isn't alive
    ///
    /// # Returns
    /// A vector of cloned resources, or an error if any resource fails to clone because it doesn't exist
    pub fn clone_resources(&mut self) -> Vec<Mutex<Resource>> {
        self.alive_mut().unwrap().clone_resources()
    }

    /// Clones only the resources in `resources` of `self`, panicks if self isn't alive
    pub fn clone_specific_resources(
        &mut self,
        resources: &[usize],
    ) -> Result<Vec<Mutex<Resource>>, ()> {
        self.alive_mut()
            .unwrap()
            .clone_specific_resources(resources)
    }

    pub fn cwd_mut(&mut self) -> &mut PathBuf {
        self.alive_mut().unwrap().cwd_mut()
    }

    pub fn extend_data_by(&mut self, amount: isize) -> Option<*mut u8> {
        self.alive_mut().unwrap().extend_data_by(amount)
    }

    pub fn die(&mut self, exit_code: usize, killed_by: Pid) {
        let Some(alive) = self.alive_mut() else {
            return;
        };

        *self = ProcessState::Zombie(unsafe { alive.die_mut(exit_code, killed_by) });
    }
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
    state: RwLock<ProcessState>,
    is_alive: AtomicBool,

    pub schedule_cleanup: AtomicBool,
    userspace_process: bool,

    next_tid: AtomicU32,
    default_priority: ContextPriority,
    allocator: Mutex<ProcessMemAllocator>,

    pub(super) threads: Mutex<Vec<Arc<Thread>>>,
    pub context_count: AtomicU32,
}

impl core::fmt::Debug for Process {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("process")
            .field("name", &self.name)
            .field("state", &self.state)
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
        let mut state = self.state_mut();
        let alive_state = state
            .alive_mut()
            .expect("attempt to allocate a new thread local for a thread that isn't alive");

        let page_table = &mut alive_state.root_page_table;
        let master_tls = alive_state.master_tls;
        let mut allocator = self.allocator.lock();

        Self::allocate_thread_local_inner(page_table, &mut *allocator, master_tls)
    }

    #[inline]
    fn allocate_thread_local_inner(
        page_table: &mut PageTable,
        allocator: &mut ProcessMemAllocator,
        master_tls: Option<(VirtAddr, usize, usize)>,
    ) -> Result<Option<(VirtAddr, TrackedAllocation)>, MapToError> {
        let Some((master_tls_addr, tls_size, tls_alignment)) = master_tls else {
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

        let size = size_of::<UThreadLocalInfo>() + tls_size;
        let tracker = allocator.allocate_tracked_guraded(size, tls_alignment, 0)?;

        let allocated_start = tracker.start();

        let (uthread_addr, tls_addr) = {
            cfg_if! {
                if #[cfg(target_arch = "x86_64")] {
                    (allocated_start + tls_size, allocated_start)
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
                        thread_local_storage_size: tls_size,
                    }
                } else if #[cfg(target_arch = "aarch64")] {
                    UThreadLocalInfo {
                        thread_local_storage_ptr: unsafe { NonNull::new_unchecked(tls_addr.into_ptr()) },
                        thread_local_storage_size: tls_size,
                    }
                } else {
                    compile_error!("TLS placement not implemented for the current architecture")
                }
            }
        };

        let uthread_bytes: [u8; size_of::<UThreadLocalInfo>()] =
            unsafe { core::mem::transmute(uthread_info) };
        copy_to_userspace(page_table, uthread_addr, &uthread_bytes);
        userspace_copy_within(page_table, master_tls_addr, tls_addr, tls_size);

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

    const fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        default_priority: ContextPriority,
        root_page_table: PhysPageTable,
        cwd: Box<PathBuf>,
        data_break: VirtAddr,
        master_tls: Option<(VirtAddr, usize, usize)>,
        allocator: ProcessMemAllocator,
        userspace_process: bool,
    ) -> Self {
        Self {
            name,
            pid,

            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            schedule_cleanup: AtomicBool::new(false),

            threads: Mutex::new(Vec::new()),

            next_tid: AtomicU32::new(1),
            context_count: AtomicU32::new(1),
            default_priority,

            state: RwLock::new(ProcessState::Alive(AliveProcess {
                root_page_table: ManuallyDrop::new(root_page_table),
                resources: ResourceManager::new(),
                master_tls,
                data_pages: 0,
                data_start: data_break,
                data_break,
                cwd,
            })),
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
        master_tls: Option<(VirtAddr, usize, usize)>,
        default_priority: ContextPriority,
        userspace_process: bool,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<(Arc<Self>, Arc<Thread>), MapToError> {
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

        let thread = Arc::new(Self::create_thread_from_status(
            &process, 0, context, None, to_track,
        ));
        process.threads.lock().push(thread.clone());

        Ok((process, thread))
    }

    /// Creates a new thread from a CPU status giving it a `cid` and everything
    fn create_thread_from_status(
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

    /// Creates a new thread from a CPU status giving it a `cid` and everything
    /// adds to the process's context count so it tracks this thread
    pub fn add_thread_to_process(
        process: &Arc<Process>,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        priority: Option<ContextPriority>,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<(Arc<Thread>, Tid), MapToError> {
        let context_id = process.next_tid.fetch_add(1, Ordering::SeqCst);
        let thread = Self::create_thread_from_process_owned(
            process,
            context_id,
            entry_point,
            argument_ptr,
            priority,
            custom_stack_size,
        )
        .map(|thread| Arc::new(thread))?;
        process.threads.lock().push(thread.clone());
        Ok((thread, context_id))
    }

    /// Creates a new thread for a given process
    /// doesn't add to the process's thread list so the thread is owned by the caller
    pub fn create_thread_from_process_owned(
        process: &Arc<Process>,
        context_id: Tid,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        priority: Option<ContextPriority>,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<Thread, MapToError> {
        let user_stack_tracker = process.allocate_stack(custom_stack_size)?;
        let kernel_stack_tracker = process.allocate_stack(custom_stack_size)?;
        let tls = process.allocate_thread_local()?;

        let (tls_addr, tls_tracker) = match tls {
            Some((tls_addr, tracker)) => (tls_addr, Some(tracker)),
            None => (VirtAddr::null(), None),
        };

        let mut write_guard = process.state_mut();
        let state = write_guard
            .alive_mut()
            .expect("tried to create a thread in a process that is not alive");
        let page_table = &mut state.root_page_table;

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

        let thread =
            Self::create_thread_from_status(process, context_id, cpu_status, priority, to_track);
        process.context_count.fetch_add(1, Ordering::Relaxed);

        Ok(thread)
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
    ) -> Result<(Arc<Self>, Arc<Thread>), ElfError> {
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

    pub fn state<'s>(&'s self) -> RwLockReadGuard<'s, ProcessState> {
        self.state.read()
    }

    pub fn state_mut<'s>(&'s self) -> RwLockWriteGuard<'s, ProcessState> {
        self.state.write()
    }

    /// kills the process
    /// if `killed_by` is `None` the process will be killed by itself
    pub fn kill(&self, exit_code: usize, killed_by: Option<Pid>) {
        let pid = self.pid();
        let killed_by = killed_by.unwrap_or(pid);

        let threads = self.threads.lock();
        let mut state = self.state.write();

        state.die(exit_code, killed_by);

        let current_thread = thread::current();
        let current_tid = current_thread.tid();
        let current_pid = current_thread.process().pid();

        let killing_self = current_pid == pid;

        for thread in &*threads {
            let tid = thread.tid();
            // we don't have to wait for self to exit
            if killing_self && current_tid == tid {
                continue;
            }

            thread.mark_dead(true);

            // wait for the thread to exit
            while thread.status_mut().is_running() {
                thread::current::yield_now();
                core::hint::spin_loop();
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

        // for some reason a thread yield may happen here sow e want to make sure everything is dropped before the process is unswitchable to
        // i actually have no idea why a thread yield would happen here...
        drop(state);
        drop(threads);
        self.is_alive.store(false, Ordering::Release);
        current_thread.mark_dead(true);
    }

    pub(super) fn cleanup(&self) -> (ProcessInfo, Option<PhysPageTable>) {
        let mut page_table = None;

        if self
            .schedule_cleanup
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::Acquire)
            .is_ok()
        {
            let mut state = self.state_mut();
            let zombie = state
                .zombie_mut()
                .expect("attempt to cleanup an alive process");

            page_table = Some(unsafe { ManuallyDrop::take(&mut zombie.root_page_table) });
        }

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

    pub last_resource_id: usize,
    pub exit_code: usize,
    pub at: VirtAddr,
    pub stack_addr: VirtAddr,

    pub killed_by: Pid,
    pub data_start: VirtAddr,
    pub data_break: VirtAddr,
    pub is_alive: bool,
}

impl From<&Process> for ProcessInfo {
    fn from(process: &Process) -> Self {
        let at = process.at();
        let stack_addr = process.stack_at();

        let state = process.state();

        let (exit_code, data_start, data_break, killed_by, last_resource_id) = match &*state {
            ProcessState::Alive(AliveProcess {
                data_start,
                data_break,
                resources,
                ..
            }) => (0, *data_start, *data_break, 0, resources.next_ri()),
            ProcessState::Zombie(ZombieProcess {
                data_start,
                data_break,
                exit_code,
                killed_by,
                last_resource_id,
                ..
            }) => (
                *exit_code,
                *data_start,
                *data_break,
                *killed_by,
                *last_resource_id,
            ),
        };

        let is_alive = process.is_alive();
        let ppid = process.ppid.load(core::sync::atomic::Ordering::Relaxed);
        let name = process.name().clone();

        Self {
            ppid,
            pid: process.pid(),
            name,
            last_resource_id,
            exit_code,
            at,
            stack_addr,

            killed_by,
            data_start,
            data_break,
            is_alive,
        }
    }
}

/// Returns [`ProcessInfo`] for the process with the given PID.
pub fn getinfo(pid: Pid) -> Option<ProcessInfo> {
    scheduler::find(|p| p.pid() == pid, |t| ProcessInfo::from(&**t))
}
