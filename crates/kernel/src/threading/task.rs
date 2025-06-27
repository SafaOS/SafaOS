use core::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    sync::atomic::{AtomicBool, AtomicU32},
};

use crate::utils::locks::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use crate::utils::types::Name;
use alloc::{boxed::Box, vec::Vec};
use safa_utils::abi::raw::processes::AbiStructures;
use serde::Serialize;

use crate::{
    arch::threading::CPUStatus,
    debug, eve,
    memory::{
        align_up, frame_allocator,
        paging::{Page, PhysPageTable, PAGE_SIZE},
    },
    utils::{
        elf::{Elf, ElfError},
        io::Readable,
        path::{Path, PathBuf},
    },
    VirtAddr,
};

use super::{
    resources::{Resource, ResourceManager},
    Pid,
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
    unsafe fn die_mut(&mut self, exit_code: usize, killed_by: Pid) -> ZombieTask { unsafe {
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
    }}
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

pub struct Task {
    /// constant
    pub pid: Pid,
    /// Task may change it's parent pid
    pub ppid: AtomicU32,
    state: RwLock<TaskState>,
    name: Name,
    /// context must only be changed by the scheduler, so it is not protected by a lock
    context: UnsafeCell<CPUStatus>,
    is_alive: AtomicBool,
}

impl Task {
    /// Creates a new task
    /// # Panics
    /// if `cwd` or `name` have a length greater than 128 or 64 bytes respectively
    pub fn new(
        name: Name,
        pid: Pid,
        ppid: Pid,
        cwd: Box<PathBuf>,
        root_page_table: PhysPageTable,
        context: CPUStatus,
        data_break: VirtAddr,
    ) -> Self {
        let data_break = VirtAddr::from(align_up(data_break.into_raw(), PAGE_SIZE));

        Self {
            name,
            pid,
            ppid: AtomicU32::new(ppid),
            is_alive: AtomicBool::new(true),
            context: UnsafeCell::new(context),
            state: RwLock::new(TaskState::Alive(AliveTask {
                root_page_table: ManuallyDrop::new(root_page_table),
                resources: ResourceManager::new(),
                data_pages: 0,
                data_start: data_break,
                data_break,
                cwd,
            })),
        }
    }

    /// Creates a new task from an elf
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
            CPUStatus::create(&mut page_table, args, env, structures, entry_point, true)?
        };
        Ok(Self::new(
            name, pid, ppid, cwd, page_table, context, data_break,
        ))
    }

    pub fn name(&self) -> &Name {
        &self.name
    }

    pub fn state<'s>(&'s self) -> Option<RwLockReadGuard<'s, TaskState>> {
        self.state.try_read()
    }

    pub fn state_mut<'s>(&'s self) -> Option<RwLockWriteGuard<'s, TaskState>> {
        self.state.try_write()
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

    pub unsafe fn set_context(&self, context: CPUStatus) { unsafe {
        *self.context.get() = context;
    }}

    pub fn context(&self) -> &CPUStatus {
        unsafe { &*self.context.get() }
    }

    fn at(&self) -> VirtAddr {
        unsafe { (*self.context.get()).at() }
    }

    fn stack_at(&self) -> VirtAddr {
        unsafe { (*self.context.get()).stack_at() }
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

        let state = task.state().unwrap();

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
