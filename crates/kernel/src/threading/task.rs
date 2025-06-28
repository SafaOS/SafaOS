use core::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use crate::{memory::paging::MapToError, threading::cpu_context, utils::types::Name};
use crate::{
    threading::cpu_context::Context,
    utils::locks::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard},
};
use alloc::{boxed::Box, vec::Vec};
use safa_utils::abi::raw::processes::AbiStructures;
use serde::Serialize;

use crate::{
    VirtAddr,
    arch::threading::CPUStatus,
    debug, eve,
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

pub struct AliveTask {
    root_page_table: ManuallyDrop<PhysPageTable>,
    resources: ResourceManager,

    data_pages: usize,
    data_start: VirtAddr,
    data_break: VirtAddr,

    cwd: Box<PathBuf>,
}

pub struct ZombieTask {
    exit_code: usize,
    killed_by: Pid,

    data_start: VirtAddr,
    data_break: VirtAddr,

    last_resource_id: usize,
    cwd: Box<PathBuf>,
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
            let root_page_table = ManuallyDrop::take(&mut self.root_page_table);
            eve::add_cleanup(root_page_table);
            ZombieTask {
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

pub(super) struct CPUContexts {
    contexts: Vec<Context>,
    current_context_index: usize,
    next_cid: cpu_context::Cid,
}

impl CPUContexts {
    pub fn create(status: CPUStatus) -> Self {
        let context = Context::new(0, status);
        Self::new(context)
    }

    pub fn new(root_context: Context) -> Self {
        Self {
            contexts: alloc::vec![root_context],
            current_context_index: 0,
            next_cid: 1,
        }
    }

    pub fn current_mut(&mut self) -> &mut Context {
        &mut self.contexts[self.current_context_index]
    }

    /// Allocate a new context and add it to the list of contexts given an entry point.
    /// Returns the id of the new context.
    pub fn allocate_add(
        &mut self,
        root_page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        userspace: bool,
    ) -> Result<cpu_context::Cid, MapToError> {
        let cid = self.next_cid;
        self.next_cid += 1;

        let cpu_status = unsafe {
            CPUStatus::create_child(
                root_page_table,
                entry_point,
                cid,
                argument_ptr.into_ptr::<()>(),
                userspace,
            )?
        };

        self.contexts.push(Context::new(cid, cpu_status));
        Ok(cid)
    }

    pub fn advance_swap(&mut self, current_status: CPUStatus) -> Option<&mut Context> {
        self.current_mut().set_cpu_status(current_status);

        if self.current_context_index + 1 < self.contexts.len() {
            self.current_context_index += 1;
            Some(self.current_mut())
        } else {
            self.current_context_index = 0;
            None
        }
    }

    pub fn exit_current(&mut self) -> bool {
        self.contexts.swap_remove(self.current_context_index);
        self.contexts.is_empty()
    }
}

pub struct Task {
    name: Name,
    /// constant
    pid: Pid,
    /// Task may change it's parent pid
    ppid: AtomicU32,
    state: RwLock<TaskState>,
    cpu_contexts: UnsafeCell<CPUContexts>,
    is_alive: AtomicBool,
    userspace_task: bool,
}

impl Task {
    pub const fn pid(&self) -> Pid {
        self.pid
    }

    pub fn ppid(&self) -> Pid {
        self.ppid.load(Ordering::Relaxed)
    }

    pub fn ppid_atomic(&self) -> &AtomicU32 {
        &self.ppid
    }

    pub(super) const fn set_pid(&mut self, pid: Pid) {
        self.pid = pid;
    }

    /// Creates a new task
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
        userspace_task: bool,
    ) -> Self {
        let data_break = VirtAddr::from(align_up(data_break.into_raw(), PAGE_SIZE));
        let cpu_contexts = CPUContexts::create(status);

        Self {
            name,
            pid,
            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            cpu_contexts: UnsafeCell::new(cpu_contexts),
            state: RwLock::new(TaskState::Alive(AliveTask {
                root_page_table: ManuallyDrop::new(root_page_table),
                resources: ResourceManager::new(),
                data_pages: 0,
                data_start: data_break,
                data_break,
                cwd,
            })),
            userspace_task,
        }
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
        structures: AbiStructures,
    ) -> Result<Self, ElfError> {
        let entry_point = elf.header().entry_point;
        let mut page_table = PhysPageTable::create()?;
        let data_break = elf.load_exec(&mut page_table)?;

        let context = unsafe {
            CPUStatus::create_root(&mut page_table, args, env, structures, entry_point, true)?
        };
        Ok(Self::new(
            name, pid, ppid, cwd, page_table, context, data_break, true,
        ))
    }

    pub fn name(&self) -> &Name {
        &self.name
    }

    pub fn state<'s>(&'s self) -> RwLockReadGuard<'s, TaskState> {
        self.state.read()
    }

    pub fn state_mut<'s>(&'s self) -> RwLockWriteGuard<'s, TaskState> {
        self.state.write()
    }

    /// kills the task
    /// if `killed_by` is `None` the task will be killed by itself
    pub fn kill(&self, exit_code: usize, killed_by: Option<Pid>) {
        let mut state = self.state.write();
        let killed_by = killed_by.unwrap_or(self.pid);

        state.die(exit_code, killed_by);
        self.is_alive
            .store(false, core::sync::atomic::Ordering::Relaxed);

        debug!(
            Task,
            "Task {} ({}) TERMINATED with code {} by {}",
            self.pid,
            self.name(),
            exit_code,
            killed_by
        );
    }

    pub(super) unsafe fn cpu_contexts(&self) -> &mut CPUContexts {
        unsafe { &mut *self.cpu_contexts.get() }
    }

    pub fn append_context(
        &self,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
    ) -> Result<cpu_context::Cid, MapToError> {
        let mut state_mut = self.state.write();
        let alive = state_mut
            .alive_mut()
            .expect("attempt to spawn a thread on a dead Task (process)");
        let page_table = &mut alive.root_page_table;

        unsafe {
            let contexts = self.cpu_contexts();
            contexts.allocate_add(page_table, entry_point, argument_ptr, self.userspace_task)
        }
    }

    fn at(&self) -> VirtAddr {
        VirtAddr::null()
    }

    fn stack_at(&self) -> VirtAddr {
        VirtAddr::null()
    }

    pub(super) fn is_alive(&self) -> bool {
        self.is_alive.load(core::sync::atomic::Ordering::Relaxed)
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
            pid: task.pid,
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
