use core::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{
    memory::paging::MapToError,
    threading::{
        cpu_context::{Cid, ContextPriority, Thread},
        this_thread,
    },
    utils::types::Name,
};
use crate::{
    threading::cpu_context::ContextStatus,
    utils::locks::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard},
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use safa_utils::abi::raw::processes::AbiStructures;
use serde::Serialize;

use crate::{
    VirtAddr,
    arch::threading::CPUStatus,
    debug,
    memory::{
        align_up, frame_allocator,
        paging::{PAGE_SIZE, Page, PhysPageTable},
    },
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::{Path, PathBuf},
    },
};

use super::{
    Pid,
    resources::{Resource, ResourceManager},
};
#[derive(Debug)]
pub struct AliveTask {
    root_page_table: ManuallyDrop<PhysPageTable>,
    resources: ResourceManager,

    data_pages: usize,
    data_start: VirtAddr,
    data_break: VirtAddr,

    cwd: Box<PathBuf>,
}
#[derive(Debug)]
pub struct ZombieTask {
    exit_code: usize,
    killed_by: Pid,

    data_start: VirtAddr,
    data_break: VirtAddr,

    last_resource_id: usize,
    cwd: Box<PathBuf>,
    root_page_table: ManuallyDrop<PhysPageTable>,
}

impl AliveTask {
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

    /// Clones only the resources in `resources` of `self`, panicks if self isn't alive
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

        let frame = frame_allocator::allocate_frame()?;

        unsafe {
            self.root_page_table
                .map_to(
                    new_page,
                    frame,
                    EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
                )
                .ok()?;
        }

        let addr = frame.virt_addr();
        let ptr = addr.into_ptr::<u8>();
        let slice = unsafe { core::slice::from_raw_parts_mut(ptr, PAGE_SIZE) };

        slice.fill(0xBB);
        self.data_pages += 1;
        Some(addr)
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
            let pages = crate::memory::align_up(amount - usable_bytes, PAGE_SIZE) / PAGE_SIZE;

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
    unsafe fn die_mut(&mut self, exit_code: usize, killed_by: Pid) -> ZombieTask {
        unsafe {
            ZombieTask {
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

impl ZombieTask {
    pub fn cwd<'s>(&'s self) -> Path<'s> {
        self.cwd.as_path()
    }
}
#[derive(Debug)]
pub enum TaskState {
    Alive(AliveTask),
    Zombie(ZombieTask),
}

impl TaskState {
    fn zombie(&self) -> Option<&ZombieTask> {
        match self {
            TaskState::Zombie(zombie) => Some(zombie),
            TaskState::Alive { .. } => None,
        }
    }

    fn zombie_mut(&mut self) -> Option<&mut ZombieTask> {
        match self {
            TaskState::Zombie(zombie) => Some(zombie),
            TaskState::Alive { .. } => None,
        }
    }

    fn alive(&self) -> Option<&AliveTask> {
        match self {
            TaskState::Alive(alive) => Some(alive),
            TaskState::Zombie { .. } => None,
        }
    }

    fn alive_mut(&mut self) -> Option<&mut AliveTask> {
        match self {
            TaskState::Alive(alive) => Some(alive),
            TaskState::Zombie { .. } => None,
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
            TaskState::Alive(alive) => alive.cwd(),
            TaskState::Zombie(zombie) => zombie.cwd(),
        }
    }

    /// Clones the resources of `self`, panicks if self isn't alive
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

        *self = TaskState::Zombie(unsafe { alive.die_mut(exit_code, killed_by) });
    }
    /// gets the exit code of the task
    /// returns `None` if the task is alive
    /// returns `Some(exit_code)` if the task is zombie
    pub fn exit_code(&self) -> Option<usize> {
        self.zombie().map(|zombie| zombie.exit_code)
    }
}

pub struct Task {
    name: Name,
    /// constant
    pid: UnsafeCell<Pid>,
    /// Task may change it's parent pid
    ppid: AtomicU32,
    state: RwLock<TaskState>,
    is_alive: AtomicBool,

    pub schedule_cleanup: AtomicBool,
    userspace_task: bool,

    next_cid: AtomicU32,
    default_priority: ContextPriority,

    threads: Mutex<Vec<Arc<Thread>>>,
    pub context_count: AtomicU32,
}

impl core::fmt::Debug for Task {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Task")
            .field("name", &self.name)
            .field("state", &self.state)
            .field("pid", &self.pid)
            .field("ppid", &self.ppid)
            .field("is_alive", &self.is_alive)
            .finish()
    }
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Task {
    pub const fn pid(&self) -> Pid {
        unsafe { *self.pid.get() }
    }

    pub fn ppid(&self) -> Pid {
        self.ppid.load(Ordering::Relaxed)
    }

    pub fn ppid_atomic(&self) -> &AtomicU32 {
        &self.ppid
    }

    pub(super) const unsafe fn set_pid(&self, pid: Pid) {
        unsafe {
            self.pid.get().write(pid);
        }
    }

    /// Creates a new task returning a combination of the task and the main thread
    /// # Panics
    /// if `cwd` or `name` have a length greater than 128 or 64 bytes respectively
    pub fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        cwd: Box<PathBuf>,
        root_page_table: PhysPageTable,
        status: CPUStatus,
        data_break: VirtAddr,
        default_priority: ContextPriority,
        userspace_task: bool,
    ) -> (Arc<Self>, Arc<Thread>) {
        let data_break = VirtAddr::from(align_up(data_break.into_raw(), PAGE_SIZE));

        let task = Arc::new(Self {
            name,
            pid: UnsafeCell::new(pid),

            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            schedule_cleanup: AtomicBool::new(false),

            threads: Mutex::new(Vec::new()),

            next_cid: AtomicU32::new(1),
            context_count: AtomicU32::new(1),
            default_priority,

            state: RwLock::new(TaskState::Alive(AliveTask {
                root_page_table: ManuallyDrop::new(root_page_table),
                resources: ResourceManager::new(),
                data_pages: 0,
                data_start: data_break,
                data_break,
                cwd,
            })),
            userspace_task,
        });

        let thread = Arc::new(Self::create_thread_from_status(&task, 0, status, None));
        task.threads.lock().push(thread.clone());

        (task, thread)
    }

    /// Creates a new thread from a CPU status giving it a `cid` and everything
    fn create_thread_from_status(
        task: &Arc<Task>,
        cid: Cid,
        cpu_status: CPUStatus,
        priority: Option<ContextPriority>,
    ) -> Thread {
        Thread::new(
            cid,
            cpu_status,
            task,
            priority.unwrap_or(task.default_priority),
        )
    }

    /// Creates a new thread from a CPU status giving it a `cid` and everything
    /// adds to the task's context count so it tracks this thread
    pub fn add_thread_to_task(
        task: &Arc<Task>,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        priority: Option<ContextPriority>,
    ) -> Result<(Arc<Thread>, Cid), MapToError> {
        let context_id = task.next_cid.fetch_add(1, Ordering::SeqCst);
        let thread = Self::create_thread_from_task_owned(
            task,
            context_id,
            entry_point,
            argument_ptr,
            priority,
        )
        .map(|thread| Arc::new(thread))?;
        task.threads.lock().push(thread.clone());
        Ok((thread, context_id))
    }

    /// Creates a new thread for a given task
    /// doesn't add to the task's thread list so the thread is owned by the caller
    pub fn create_thread_from_task_owned(
        task: &Arc<Task>,
        context_id: Cid,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        priority: Option<ContextPriority>,
    ) -> Result<Thread, MapToError> {
        let mut write_guard = task.state_mut();
        let state = write_guard
            .alive_mut()
            .expect("tried to create a thread in a task that is not alive");
        let page_table = &mut state.root_page_table;

        let cpu_status = unsafe {
            CPUStatus::create_child(
                page_table,
                entry_point,
                context_id,
                argument_ptr.into_ptr::<()>(),
                task.userspace_task,
            )?
        };

        let thread = Self::create_thread_from_status(task, context_id, cpu_status, priority);
        task.context_count.fetch_add(1, Ordering::Relaxed);

        Ok(thread)
    }

    /// Creates a new task from an elf
    /// that task is assumed to be in the userspace
    pub fn from_elf<T: Readable>(
        name: Name,
        pid: Pid,
        ppid: Pid,
        cwd: Box<PathBuf>,
        elf: Elf<T>,
        args: &[&str],
        env: &[&[u8]],
        default_priority: ContextPriority,
        structures: AbiStructures,
    ) -> Result<(Arc<Self>, Arc<Thread>), ElfError> {
        let entry_point = elf.header().entry_point;
        let mut page_table = PhysPageTable::create()?;
        let data_break = elf.load_exec(&mut page_table)?;

        let context = unsafe {
            CPUStatus::create_root(&mut page_table, args, env, structures, entry_point, true)?
        };

        Ok(Self::new(
            name,
            pid,
            ppid,
            cwd,
            page_table,
            context,
            data_break,
            default_priority,
            true,
        ))
    }

    pub fn name(&self) -> &Name {
        &self.name
    }

    pub fn state<'s>(&'s self) -> RwLockReadGuard<'s, TaskState> {
        self.state.read()
    }

    pub fn try_state<'s>(&'s self) -> Option<RwLockReadGuard<'s, TaskState>> {
        self.state.try_read()
    }

    pub fn state_mut<'s>(&'s self) -> RwLockWriteGuard<'s, TaskState> {
        self.state.write()
    }

    /// kills the task
    /// if `killed_by` is `None` the task will be killed by itself
    pub fn kill(&self, exit_code: usize, killed_by: Option<Pid>) {
        let threads = self.threads.lock();
        let mut state = self.state.write();

        let killed_by = killed_by.unwrap_or(self.pid());
        let pid = self.pid();

        state.die(exit_code, killed_by);

        for thread in &*threads {
            thread.mark_dead(true);
        }

        let this_thread = this_thread();
        let this_cid = unsafe { this_thread.context().cid() };
        let this_pid = this_thread.task().pid();
        let killing_self = this_pid == pid;

        for thread in &*threads {
            let context = unsafe { thread.context() };
            let cid = context.cid();
            // we don't have to wait for self to exit
            if killing_self && this_cid == cid {
                continue;
            }

            // wait for the thread to exit
            while context.status() == ContextStatus::Running {
                core::hint::spin_loop();
            }
        }

        self.is_alive.store(false, Ordering::Release);

        debug!(
            Task,
            "Task {} ({}) TERMINATED with code {} by {}",
            pid,
            self.name(),
            exit_code,
            killed_by
        );
    }

    pub(super) fn cleanup(&self) -> (TaskInfo, Option<PhysPageTable>) {
        let mut page_table = None;

        if self
            .schedule_cleanup
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::Acquire)
            .is_ok()
        {
            let mut state = self.state_mut();
            let zombie = state
                .zombie_mut()
                .expect("attempt to cleanup an alive task");

            page_table = Some(unsafe { ManuallyDrop::take(&mut zombie.root_page_table) });
        }

        (TaskInfo::from(self), page_table)
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

#[derive(Serialize, Debug, Clone)]
#[repr(C)]
pub struct TaskInfo {
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

impl From<&Task> for TaskInfo {
    fn from(task: &Task) -> Self {
        let at = task.at();
        let stack_addr = task.stack_at();

        let state = task.state();

        let (exit_code, data_start, data_break, killed_by, last_resource_id) = match &*state {
            TaskState::Alive(AliveTask {
                data_start,
                data_break,
                resources,
                ..
            }) => (0, *data_start, *data_break, 0, resources.next_ri()),
            TaskState::Zombie(ZombieTask {
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

        let is_alive = task.is_alive();
        let ppid = task.ppid.load(core::sync::atomic::Ordering::Relaxed);
        let name = task.name().clone();

        Self {
            ppid,
            pid: task.pid(),
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
