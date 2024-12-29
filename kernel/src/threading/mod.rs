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

use core::arch::asm;
use lazy_static::lazy_static;
use processes::{
    AliveProcessState, Process, ProcessFlags, ProcessInfo, ProcessState, ProcessStatus,
};

use alloc::string::String;
use spin::Mutex;

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
pub type ProcessItem = Process;

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
        add_process(process);

        // getting the context of the first process
        // like this so the scheduler read lock is released
        let context = with_current(|process| process.context);

        debug!(Scheduler, "INITED ...");
        unsafe { restore_cpu_status(&context) }
    }

    /// gets a mutable reference to the current process
    fn current(&mut self) -> &mut Process {
        unsafe { self.processes.current_mut().unwrap_unchecked() }
    }

    /// context switches into next process, takes current context outputs new context
    pub unsafe fn switch(&mut self, context: CPUStatus) -> CPUStatus {
        unsafe { asm!("cli") }

        self.current().context = context;
        self.current().status = ProcessStatus::Waiting;

        for process in self.processes.continue_iter() {
            if process.status == ProcessStatus::Waiting {
                process.status = ProcessStatus::Running;
                break;
            }
        }

        self.current().context
    }

    /// appends a process to the end of the scheduler Processes list
    /// returns the pid of the added process
    pub fn add_process(&mut self, mut process: Process) -> usize {
        let pid = self.next_pid;
        process.pid = pid;
        process.status = ProcessStatus::Waiting;
        self.next_pid += 1;
        self.processes.push(process);

        debug!(Scheduler, "process with pid {} CREATED ...", pid);
        pid
    }

    /// finds a process where executing `condition` on returns true, then executes `then` on it
    /// returns the result of `then` if a process was found
    fn find<C, T, R>(&self, condition: C, mut then: T) -> Option<R>
    where
        C: Fn(&Process) -> bool,
        T: FnMut(&Process) -> R,
    {
        for process in self.processes.clone_iter() {
            if condition(process) {
                return Some(then(process));
            }
        }

        None
    }

    /// iterates through all processes and executes `then` on each of them
    /// executed on all processes
    pub fn for_each<T>(&mut self, mut then: T)
    where
        T: FnMut(&mut Process),
    {
        for process in self.processes.clone_iter_mut() {
            then(process);
        }
    }

    /// iterates through all processes and executes `then` on each of them
    /// if then returns false it breaks the loop
    /// executed on all processes
    pub fn while_each<T>(&mut self, mut then: T)
    where
        T: FnMut(&mut Process) -> bool,
    {
        for process in self.processes.clone_iter_mut() {
            if !then(process) {
                break;
            }
        }
    }

    /// attempt to remove a process where executing `condition` on returns true, returns the removed process info
    pub fn remove(&mut self, condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
        self.processes
            .remove_where(|process| condition(process))
            .map(|process| process.info())
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
        .try_lock()
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

    unsafe { SCHEDULER.lock().switch(context) }
}

lazy_static! {
    static ref SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());
}

/// acquires lock on scheduler and executes `then` on the current process
fn with_current<T, R>(then: T) -> R
where
    T: FnOnce(&mut Process) -> R,
{
    then(SCHEDULER.lock().current())
}

/// acquires lock on scheduler and executes `then` on the current process state
fn with_current_state<T, R>(then: T) -> R
where
    T: FnOnce(&mut AliveProcessState) -> R,
{
    with_current(|process| match &mut process.state {
        ProcessState::Alive(state) => then(state),
        _ => unreachable!(),
    })
}

/// acquires lock on scheduler and finds a process where executing `condition` on returns true, then executes `then` on it
/// returns the result of `then` if a process was found
fn find<C, T, R>(condition: C, then: T) -> Option<R>
where
    C: Fn(&Process) -> bool,
    T: FnMut(&Process) -> R,
{
    SCHEDULER.lock().find(condition, then)
}

/// acquires lock on scheduler
/// executes `then` on each process
fn for_each<T>(then: T)
where
    T: FnMut(&mut Process),
{
    SCHEDULER.lock().for_each(then)
}

/// acquires lock on scheduler
/// executes `then` on each process until it returns false
fn while_each<T>(then: T)
where
    T: FnMut(&mut Process) -> bool,
{
    SCHEDULER.lock().while_each(then)
}

/// acquires lock on scheduler and returns the number of processes
pub fn pcount() -> usize {
    SCHEDULER.lock().processes_count()
}

/// acquires lock on scheduler and adds a process to it
fn add_process(process: Process) -> usize {
    SCHEDULER.lock().add_process(process)
}

/// acquires lock on scheduler and removes a process from it where `condition` on the process returns true
fn remove(condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
    SCHEDULER.lock().remove(condition)
}
