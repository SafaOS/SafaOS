use core::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    sync::atomic::{AtomicBool, AtomicUsize},
};

use alloc::string::String;
use serde::Serialize;
use spin::{RwLock, RwLockReadGuard, RwLockWriteGuard};

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
    },
    VirtAddr,
};

use super::{resources::ResourceManager, Pid};

pub enum TaskState {
    Alive {
        root_page_table: ManuallyDrop<PhysPageTable>,
        resources: ResourceManager,

        data_pages: usize,
        data_start: VirtAddr,
        data_break: VirtAddr,

        cwd: String,
    },
    Zombie {
        exit_code: usize,
        killed_by: Pid,

        data_start: VirtAddr,
        data_break: VirtAddr,

        last_resource_id: usize,
        cwd: String,
    },
}

impl TaskState {
    pub fn resource_manager(&self) -> Option<&ResourceManager> {
        match self {
            TaskState::Alive { resources, .. } => Some(resources),
            TaskState::Zombie { .. } => None,
        }
    }

    pub fn resource_manager_mut(&mut self) -> Option<&mut ResourceManager> {
        match self {
            TaskState::Alive { resources, .. } => Some(resources),
            TaskState::Zombie { .. } => None,
        }
    }

    pub fn cwd(&self) -> &str {
        match self {
            TaskState::Alive { cwd, .. } => cwd,
            TaskState::Zombie { cwd, .. } => cwd,
        }
    }

    pub fn cwd_mut(&mut self) -> &mut String {
        match self {
            TaskState::Alive { cwd, .. } => cwd,
            TaskState::Zombie { cwd, .. } => cwd,
        }
    }

    fn page_extend_data(&mut self) -> Option<VirtAddr> {
        match self {
            TaskState::Alive {
                data_start,
                data_pages,
                root_page_table,
                ..
            } => {
                use crate::memory::paging::EntryFlags;

                let page_end = *data_start + PAGE_SIZE * *data_pages;
                let new_page = Page::containing_address(page_end);

                let frame = frame_allocator::allocate_frame()?;

                root_page_table
                    .map_to(
                        new_page,
                        frame,
                        EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
                    )
                    .ok()?;

                let addr = frame.virt_addr();
                let ptr = addr as *mut u8;
                let slice = unsafe { core::slice::from_raw_parts_mut(ptr, PAGE_SIZE) };

                slice.fill(0xAA);
                *data_pages += 1;
                Some(addr)
            }
            TaskState::Zombie { .. } => None,
        }
    }

    fn page_unextend_data(&mut self) -> Option<VirtAddr> {
        match self {
            TaskState::Alive {
                data_start,
                data_pages,
                root_page_table,
                ..
            } => {
                if *data_pages == 0 {
                    return Some(*data_start);
                }

                let page_end = *data_start + PAGE_SIZE * *data_pages;

                let page = Page::containing_address(page_end - PAGE_SIZE);
                root_page_table.unmap(page);

                *data_pages -= 1;
                Some(page_end - PAGE_SIZE)
            }
            TaskState::Zombie { .. } => None,
        }
    }

    pub fn extend_data_by(&mut self, amount: isize) -> Option<*mut u8> {
        let is_negative = amount.is_negative();
        let amount = amount.unsigned_abs();

        let pages = crate::memory::align_up(amount, PAGE_SIZE) / PAGE_SIZE;

        if is_negative {
            for _ in 0..pages {
                self.page_unextend_data()?;
            }
        } else {
            for _ in 0..pages {
                self.page_extend_data()?;
            }
        }

        match self {
            TaskState::Alive { data_break, .. } => {
                if is_negative {
                    *data_break -= amount;
                } else {
                    *data_break += amount;
                }

                Some(*data_break as *mut u8)
            }
            TaskState::Zombie { .. } => None,
        }
    }

    pub fn die(&mut self, exit_code: usize, killed_by: Pid) {
        match self {
            TaskState::Alive {
                cwd,
                data_start,
                data_break,
                resources,
                root_page_table,
                ..
            } => {
                let last_resource_id = resources.next_ri();

                let root_page_table = unsafe { ManuallyDrop::take(root_page_table) };
                eve::add_cleanup(root_page_table);

                *self = TaskState::Zombie {
                    exit_code,
                    killed_by,
                    data_start: *data_start,
                    data_break: *data_break,
                    last_resource_id,
                    cwd: core::mem::take(cwd),
                };
            }
            TaskState::Zombie { .. } => {}
        }
    }
    /// gets the exit code of the task
    /// returns `None` if the task is alive
    /// returns `Some(exit_code)` if the task is zombie
    /// can be used to check if the task is alive
    pub fn exit_code(&self) -> Option<usize> {
        match &self {
            TaskState::Alive { .. } => None,
            TaskState::Zombie { exit_code, .. } => Some(*exit_code),
        }
    }
}

pub struct Task {
    /// constant
    pub pid: Pid,
    /// Task may change it's parent pid
    pub ppid: AtomicUsize,
    state: RwLock<TaskState>,
    name: String,
    /// context must only be changed by the scheduler, so it is not protected by a lock
    context: UnsafeCell<CPUStatus>,
    is_alive: AtomicBool,
}

impl Task {
    /// Creates a new task
    /// # Panics
    /// if `cwd` or `name` have a length greater than 128 or 64 bytes respectively
    pub fn new(
        name: String,
        pid: Pid,
        ppid: Pid,
        cwd: String,
        root_page_table: PhysPageTable,
        context: CPUStatus,
        data_break: VirtAddr,
    ) -> Self {
        assert!(cwd.len() < 128);
        assert!(name.len() < 64);
        let data_break = align_up(data_break, PAGE_SIZE);

        Self {
            name,
            pid,
            ppid: AtomicUsize::new(ppid),
            is_alive: AtomicBool::new(true),
            context: UnsafeCell::new(context),
            state: RwLock::new(TaskState::Alive {
                root_page_table: ManuallyDrop::new(root_page_table),
                resources: ResourceManager::new(),
                data_pages: 0,
                data_start: data_break,
                data_break,
                cwd,
            }),
        }
    }

    /// Creates a new task from an elf
    pub fn from_elf<T: Readable>(
        name: String,
        pid: Pid,
        ppid: Pid,
        cwd: String,
        elf: Elf<T>,
        args: &[&str],
    ) -> Result<Self, ElfError> {
        let entry_point = elf.header().entry_point;
        let mut page_table = PhysPageTable::create()?;
        let data_break = elf.load_exec(&mut page_table)?;

        let context = unsafe { CPUStatus::create(&mut page_table, args, entry_point, true)? };
        Ok(Self::new(
            name, pid, ppid, cwd, page_table, context, data_break,
        ))
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn state(&self) -> Option<RwLockReadGuard<TaskState>> {
        self.state.try_read()
    }

    pub fn state_mut(&self) -> Option<RwLockWriteGuard<TaskState>> {
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

    pub unsafe fn set_context(&self, context: CPUStatus) {
        *self.context.get() = context;
    }

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

#[derive(Debug, Clone)]
struct Name([u8; 64]);

impl From<[u8; 64]> for Name {
    fn from(name: [u8; 64]) -> Self {
        Self(name)
    }
}

impl serde::Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        unsafe {
            serializer.serialize_str(core::str::from_utf8_unchecked(&self.0).trim_matches('\0'))
        }
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
            TaskState::Alive {
                data_start,
                data_break,
                resources,
                ..
            } => (0, *data_start, *data_break, 0, resources.next_ri()),
            TaskState::Zombie {
                data_start,
                data_break,
                exit_code,
                killed_by,
                last_resource_id,
                ..
            } => (
                *exit_code,
                *data_start,
                *data_break,
                *killed_by,
                *last_resource_id,
            ),
        };

        let is_alive = task.is_alive();
        let ppid = task.ppid.load(core::sync::atomic::Ordering::Relaxed);

        let mut name = [0u8; 64];
        name[..task.name().len()].copy_from_slice(task.name().as_bytes());

        Self {
            ppid,
            pid: task.pid,
            name: name.into(),
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
