pub mod expose;
pub mod processes;
pub mod resources;

pub const STACK_SIZE: usize = PAGE_SIZE * 6;
pub const STACK_START: usize = 0x00007A3000000000;
pub const STACK_END: usize = STACK_START + STACK_SIZE;

pub const RING0_STACK_START: usize = 0x00007A0000000000;
pub const RING0_STACK_END: usize = RING0_STACK_START + STACK_SIZE;

pub const ENVIROMENT_START: usize = 0x00007E0000000000;
pub const ARGV_START: usize = ENVIROMENT_START + 0xA000000000;
pub const ARGV_SIZE: usize = PAGE_SIZE * 4;

use core::{arch::asm, cell::UnsafeCell};
use lazy_static::lazy_static;
use processes::{
    AliveProcessState, Process, ProcessFlags, ProcessInfo, ProcessState, ProcessStatus,
};

use alloc::{string::String, sync::Arc};
use spin::RwLock;

use crate::{
    arch::threading::{restore_cpu_status, CPUStatus},
    debug, hddm,
    memory::{
        frame_allocator::Frame,
        paging::{current_root_table, EntryFlags, MapToError, Page, PageTable, PAGE_SIZE},
    },
    utils::alloc::LinkedList,
};

/// allocates and maps an area starting from `$start` with size `$size` and returns `Result<(), MapToError>` in `$page_table`
macro_rules! alloc_map {
    ($page_table: expr, $start: ident, $size: ident) => {
        let page_table = $page_table;

        const PAGES: usize = $size / PAGE_SIZE;
        const END: usize = $start + $size;

        // allocating frames
        let mut frames: [Frame; PAGES] = [Frame::containing_address(0); PAGES];

        for i in 0..frames.len() {
            frames[i] = $crate::memory::frame_allocator::allocate_frame()
                .ok_or(MapToError::FrameAllocationFailed)?;
        }

        for frame in frames {
            let virt_addr = frame.start_address | crate::hddm();
            let byte_array = virt_addr as *mut u8;
            let byte_array = unsafe { core::slice::from_raw_parts_mut(byte_array, PAGE_SIZE) };
            byte_array.fill(0);
        }

        let start_page = Page::containing_address($start);
        let end_page = Page::containing_address(END);

        let iter = Page::iter_pages(start_page, end_page);

        for (i, page) in iter.enumerate() {
            page_table.map_to(
                page,
                frames[i],
                EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
            )?;
        }

        return Ok(());
    };
}

/// allocates and maps a stack to page_table
pub fn alloc_stack(page_table: &mut PageTable) -> Result<(), MapToError> {
    alloc_map!(page_table, STACK_START, STACK_SIZE);
}

/// allocates and maps the argv area to `page_table`
pub fn alloc_argv(page_table: &mut PageTable) -> Result<(), MapToError> {
    alloc_map!(page_table, ARGV_START, ARGV_SIZE);
}

/// allocates and maps a ring0 stack to page_table
pub fn alloc_ring0_stack(page_table: &mut PageTable) -> Result<(), MapToError> {
    alloc_map!(page_table, RING0_STACK_START, STACK_SIZE);
}

// a process is independent of the scheduler we don't want to lock it
// TODO: remove the Arc, i am aware that it is useless and pollutes the heap bringing disadvantages
// but if i remove it now things will break because they need write access to the process and the scheduler at the same time and to have access to the process they need read lock on the scheduler, an Arc helps make this temporary
pub type ProcessItem = Arc<UnsafeCell<Process>>;

pub struct Scheduler {
    processes: LinkedList<ProcessItem>,
    next_pid: usize,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            processes: LinkedList::new(),
            next_pid: 0,
        }
    }

    #[inline]
    /// inits the scheduler
    pub unsafe fn init(function: usize, name: &str) -> ! {
        debug!(Scheduler, "initing ...");
        asm!("cli");
        let page_table_addr = current_root_table() as *mut PageTable as usize - hddm();
        let process = Process::new(
            function,
            0,
            0,
            name,
            &[],
            0,
            page_table_addr,
            String::from("ram:/"),
            ProcessFlags::empty(),
        )
        .unwrap();
        SCHEDULER.write().add_process(process);

        // getting the context of the first process
        // like this so the scheduler read lock is released
        let context = SCHEDULER.read().current().context;

        debug!(Scheduler, "INITED ...");
        unsafe { restore_cpu_status(&context) }
    }

    pub fn current(&self) -> &mut Process {
        unsafe { &mut *self.processes.current().unwrap_unchecked().get() }
    }

    /// context switches into next process, takes current context outputs new context
    pub unsafe fn switch(&mut self, context: CPUStatus) -> CPUStatus {
        unsafe { asm!("cli") }

        self.current().context = context;
        self.current().status = ProcessStatus::Waiting;

        for process in self.processes.continue_iter() {
            let process = &mut *process.get();
            if process.status == ProcessStatus::Waiting {
                process.status = ProcessStatus::Running;
                break;
            }
        }

        return self.current().context;
    }

    /// appends a process to the end of the scheduler Processes list
    /// returns the pid of the added process
    pub fn add_process(&mut self, mut process: Process) -> usize {
        let pid = self.next_pid;
        process.pid = pid;
        process.status = ProcessStatus::Waiting;
        self.next_pid += 1;
        self.processes.push(Arc::new(UnsafeCell::new(process)));

        debug!(Scheduler, "process with pid {} CREATED ...", pid);
        return pid;
    }

    /// finds a process where executing `condition` on returns true, then executes `then` on it
    /// returns the result of `then` if a process was found
    pub fn find<C>(&self, condition: C) -> Option<ProcessItem>
    where
        C: Fn(&Process) -> bool,
    {
        for process in self.processes.clone_iter() {
            unsafe {
                if condition(&*process.get()) {
                    return Some(process.clone());
                }
            }
        }

        None
    }

    /// iterates through all processes and executes `then` on each of them
    pub fn for_each<T>(&self, mut then: T)
    where
        T: FnMut(&mut Process),
    {
        for process in self.processes.clone_iter() {
            unsafe {
                then(&mut *process.get());
            }
        }
    }

    /// iterates through all processes and executes `then` on each of them only if `condition` on the process returns true
    pub fn for_each_where<C, T>(&self, condition: C, mut then: T)
    where
        C: Fn(&Process) -> bool,
        T: FnMut(&mut Process),
    {
        for process in self.processes.clone_iter() {
            let process = unsafe { &mut *process.get() };
            if condition(process) {
                then(process);
            }
        }
    }

    /// iterates through all processes and executes `then` on each of them
    /// if then returns false it breaks the loop
    pub fn while_each<T>(&self, mut then: T)
    where
        T: FnMut(&mut Process) -> bool,
    {
        for process in self.processes.clone_iter() {
            let process = unsafe { &mut *process.get() };
            if !then(process) {
                break;
            }
        }
    }

    /// attempt to remove a process where executing `condition` on returns true, returns the removed process info
    pub fn remove(&mut self, condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
        unsafe {
            self.processes
                .remove_where(|process| condition(&*process.get()))
                .map(|process| (*process.get()).info())
        }
    }

    #[inline(always)]
    pub fn processes_count(&self) -> usize {
        self.processes.len()
    }

    #[inline(always)]
    /// wether or not has been properly initialized using `init`
    pub fn inited(&self) -> bool {
        self.processes.len() > 0
    }
}
#[inline(always)]
/// returns wether or not the scheduler has been initialized and is ready to be used
pub fn scheduler_ready() -> bool {
    SCHEDULER
        .try_write()
        .is_some_and(|scheduler| scheduler.inited())
}

#[inline(always)]
/// peforms a context switch using the scheduler, switching to the next process context
/// a warpper around `SCHEDULER.write().switch(context)` it also checks if the scheduler is ready
/// to be used
pub fn swtch(context: CPUStatus) -> CPUStatus {
    unsafe { asm!("cli") }
    if !scheduler_ready() {
        return context;
    }

    unsafe { SCHEDULER.write().switch(context) }
}

lazy_static! {
    static ref SCHEDULER: RwLock<Scheduler> = RwLock::new(Scheduler::new());
}

/// wrapper around `SCHEDULER.read().with_current`
fn with_current<T, R>(then: T) -> R
where
    T: FnOnce(&mut Process) -> R,
{
    unsafe {
        let process = SCHEDULER
            .read()
            .processes
            .current()
            .unwrap_unchecked()
            .clone();
        let result = then(&mut *process.get());
        result
    }
}

/// wrapper around `SCHEDULER.read().with_current_state`
fn with_current_state<T, R>(then: T) -> R
where
    T: FnOnce(&mut AliveProcessState) -> R,
{
    with_current(|process| match &mut process.state {
        ProcessState::Alive(state) => then(state),
        _ => unreachable!(),
    })
}

/// finds a process where executing `condition` on returns true, then executes `then` on it
/// gets a write lock on the process while executing `then`
/// gets a temporary read lock on the process while executing `condition` and another one on the scheduler that is released before `then` is executed
fn find<C, T, R>(condition: C, mut then: T) -> Option<R>
where
    C: Fn(&Process) -> bool,
    T: FnMut(&mut Process) -> R,
{
    let process = SCHEDULER.read().find(condition)?;
    let process = unsafe { &mut *process.get() };
    let result = then(process);
    Some(result)
}

/// wrapper around `SCHEDULER.read().for_each`
pub fn for_each<T>(then: T)
where
    T: FnMut(&mut Process),
{
    SCHEDULER.read().for_each(then)
}

/// wrapper around `SCHEDULER.read().for_each_where`
fn for_each_where<C, T>(condition: C, then: T)
where
    C: Fn(&Process) -> bool,
    T: FnMut(&mut Process),
{
    SCHEDULER.read().for_each_where(condition, then)
}

/// wrapper around `SCHEDULER.read().while_each`
fn while_each<T>(then: T)
where
    T: FnMut(&mut Process) -> bool,
{
    SCHEDULER.read().while_each(then)
}

/// wrapper around `SCHEDULER.read().processes_count`
pub fn pcount() -> usize {
    SCHEDULER.read().processes_count()
}

/// wrapper around `SCHEDULER.write().add_process`
fn add_process(process: Process) -> usize {
    SCHEDULER.write().add_process(process)
}

/// wrapper around `SCHEDULER.write().remove`
fn remove(condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
    SCHEDULER.write().remove(condition)
}
