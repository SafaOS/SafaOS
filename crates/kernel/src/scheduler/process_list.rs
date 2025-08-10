use alloc::{sync::Arc, vec::Vec};
use slab::Slab;

use crate::{
    debug,
    process::{Pid, Process, ProcessInfo},
    thread,
    utils::locks::RwLock,
};

struct ProcessList {
    processes: Vec<Arc<Process>>,
    pids: Slab<()>,
}

unsafe impl Send for ProcessList {}
unsafe impl Sync for ProcessList {}

impl ProcessList {
    pub const fn new() -> Self {
        Self {
            processes: Vec::new(),
            pids: Slab::new(),
        }
    }

    pub fn add_pid(&mut self) -> Pid {
        self.pids.insert(()) as Pid
    }

    /// appends a process to the end of the scheduler processes list
    /// returns the pid of the added process
    fn add_process(&mut self, process: Arc<Process>) -> Pid {
        let pid = process.pid();

        self.processes.push(process.clone());
        let name = process.name();

        debug!(ProcessList, "Process {} ({}) PROCESS ADDED", pid, name);
        pid
    }

    /// finds a process where executing `condition` on returns true and returns it
    fn find<C>(&self, condition: C) -> Option<&Arc<Process>>
    where
        C: Fn(&Process) -> bool,
    {
        for process in self.processes.iter() {
            if condition(process) {
                return Some(&process);
            }
        }

        None
    }

    /// iterates through all processes and executes `then` on each of them
    /// executed on all processes
    pub fn for_each<T>(&self, mut then: T)
    where
        T: FnMut(&Process),
    {
        for process in self.processes.iter() {
            then(process);
        }
    }

    /// attempt to remove a process where executing `condition` on returns true, returns the removed process info
    pub fn remove(&mut self, condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
        let mut remove_proc = || {
            let mut index = None;
            for (i, process) in self.processes.iter().enumerate() {
                if condition(process) {
                    index = Some(i);
                    break;
                }
            }
            index.map(|index| self.processes.swap_remove(index))
        };

        let process = remove_proc()?;

        let mut threads = process.threads.lock();
        for thread in &*threads {
            assert!(thread.is_dead());
            // wait for thread to exit before removing
            // will be removed on context switch at some point
            while !thread.is_removed() {
                // --> thread is removed on thread yield by the scheduler as a part of the thread list iteration
                // one thread yield should be enough
                thread::current::yield_now();
                // however maybe the thread list is in another CPU...
                core::hint::spin_loop();
            }
        }
        // Because processes are referenced by their own threads, we need to clear the threads so they drop the process reference
        // TODO: Maybe move this somewhere else
        threads.clear();

        let info = process.info();

        self.pids.remove(info.pid as usize);
        Some(info)
    }
}

static PROCESS_LIST: RwLock<ProcessList> = RwLock::new(ProcessList::new());

/// acquires lock on the process list and finds a process where executing `condition` on returns true and returns the result of `map` on that process
pub fn find<C, M, R>(condition: C, map: M) -> Option<R>
where
    C: Fn(&Process) -> bool,
    M: FnMut(&Arc<Process>) -> R,
{
    let schd = PROCESS_LIST.read();
    schd.find(condition).map(map)
}

/// acquires lock on the process list
/// executes `then` on each process
pub fn for_each<T>(then: T)
where
    T: FnMut(&Process),
{
    PROCESS_LIST.read().for_each(then)
}

/// acquires lock on the process list and adds a process to it
pub fn add_process(process: Arc<Process>) -> Pid {
    PROCESS_LIST.write().add_process(process)
}

/// returns the result of `then` if a process was found
/// acquires lock on the process list and removes a process from it where `condition` on the process returns true
pub fn remove(condition: impl Fn(&Process) -> bool) -> Option<ProcessInfo> {
    PROCESS_LIST.write().remove(condition)
}

/// Adds a new claimed pid to the process list and returns the pid
///
/// the returned pid is guaranteed to be unique and not in use by any other process
pub fn add_pid() -> Pid {
    PROCESS_LIST.write().add_pid()
}
