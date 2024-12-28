use core::slice;

use super::resources::ResourceManager;
use super::{ARGV_START, STACK_END};

use crate::memory::{align_up, copy_to_userspace, frame_allocator};
use crate::utils::elf::{Elf, ElfError};
use crate::{arch, debug, hddm, PhysAddr};

use crate::memory::paging::{self, EntryFlags, MapToError, Page, PAGE_SIZE};
use alloc::string::String;
use bitflags::bitflags;
use spin::Mutex;

use crate::{arch::threading::CPUStatus, memory::paging::PageTable};

bitflags! {
    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    pub struct ProcessFlags: u8 {
        const USERSPACE = 1 << 0;
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    Waiting,
    Running,
    Zombie,
}

#[derive(Debug)]
pub struct AliveProcessState {
    root_page_table: *mut PageTable,
    pub(super) resource_manager: Mutex<ResourceManager>,
    data_pages: usize,
    pub(super) current_dir: String,

    data_start: usize,
    data_break: usize,
}

impl AliveProcessState {
    pub fn new(current_dir: String, root_page_table_addr: PhysAddr, data_break: usize) -> Self {
        let data_break = align_up(data_break, PAGE_SIZE);
        AliveProcessState {
            root_page_table: (root_page_table_addr | hddm()) as *mut PageTable,
            resource_manager: Mutex::new(ResourceManager::new()),
            current_dir,

            data_pages: 0,
            data_break,
            data_start: data_break,
        }
    }

    #[inline(always)]
    fn data_break_actual(&self) -> usize {
        self.data_start + PAGE_SIZE * self.data_pages
    }

    fn page_extend_data(&mut self) -> Result<(), MapToError> {
        let page_end = self.data_break_actual();
        let new_page = Page::containing_address(page_end);

        let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

        unsafe {
            (*self.root_page_table).map_to(
                new_page,
                frame,
                EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
            )?
        };

        let addr = frame.start_address | hddm();
        let ptr = addr as *mut u8;
        let slice = unsafe { slice::from_raw_parts_mut(ptr, PAGE_SIZE) };

        slice.fill(0);
        self.data_pages += 1;
        Ok(())
    }

    fn page_unextend_data(&mut self) {
        let page_end = self.data_break_actual();
        let new_page = Page::containing_address(page_end);

        let frame = unsafe { (*self.root_page_table).get_frame(new_page).unwrap() };
        frame_allocator::deallocate_frame(frame);

        self.data_pages -= 1;
    }

    pub fn extend_data_by(&mut self, amount: isize) -> Result<*mut u8, MapToError> {
        if amount >= 0 {
            let amount = amount as usize;
            while self.data_break_actual() < self.data_break + amount {
                self.page_extend_data()?;
            }

            self.data_break += amount;
        } else {
            let amount = amount as usize;
            while self.data_break_actual() > self.data_break - amount {
                self.page_unextend_data();
            }

            self.data_break -= amount;
        }

        Ok(self.data_break as *mut u8)
    }
}

#[derive(Debug)]
pub struct ZombieProcessState {
    pub exit_code: usize,
    pub exit_addr: usize,
    pub exit_stack_addr: usize,
    /// the pid of the process that killed this process
    pub killed_by: usize,
    pub last_resource_id: usize,

    pub data_start: usize,
    pub data_break: usize,
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct ProcessInfo {
    pub ppid: usize,
    pub pid: usize,
    pub name: [u8; 64],
    pub status: ProcessStatus,

    pub resource_count: usize,
    pub exit_code: usize,
    pub exit_addr: usize,
    pub exit_stack_addr: usize,

    pub killed_by: usize,
    pub data_start: usize,
    pub data_break: usize,
}

#[derive(Debug)]
pub enum ProcessState {
    Zombie(ZombieProcessState),
    Alive(AliveProcessState),
}

#[derive(Debug)]
pub struct Process {
    pub ppid: usize,
    pub pid: usize,
    pub name: [u8; 64],
    pub status: ProcessStatus,
    pub context: CPUStatus,

    pub state: ProcessState,
}

unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    #[inline(always)]
    pub fn new(
        function: usize,
        ppid: usize,
        pid: usize,
        name: &str,
        argv: &[&str],
        data_start: usize,
        root_page_table_addr: usize,
        current_work_dir: String,
        flags: ProcessFlags,
    ) -> Result<Self, MapToError> {
        let name_bytes = name.as_bytes();

        let mut name = [0u8; 64];

        let len = name_bytes.len().min(64);
        name[..len].copy_from_slice(&name_bytes[..len]);

        let status = ProcessStatus::Waiting;
        let mut context = CPUStatus::default();

        let root_page_table = (root_page_table_addr | hddm()) as *mut PageTable;

        unsafe {
            let page_table = &mut *root_page_table;
            super::alloc_stack(page_table)?;
            super::alloc_ring0_stack(page_table)?;
            super::alloc_argv(page_table)?;

            if !argv.is_empty() {
                let mut start_addr = ARGV_START;
                const USIZE_BYTES: usize = size_of::<usize>();
                let argc = argv.len();

                // argc
                copy_to_userspace(page_table, start_addr, &argc.to_ne_bytes());

                // argv*
                start_addr += USIZE_BYTES;

                for arg in argv {
                    let arg = arg.as_bytes();
                    let len = arg.len();

                    copy_to_userspace(page_table, start_addr, &len.to_ne_bytes());
                    start_addr += USIZE_BYTES;

                    copy_to_userspace(page_table, start_addr, arg);
                    // null-terminate arg
                    copy_to_userspace(page_table, start_addr + len, b"\0");
                    start_addr += len + 1;
                }

                let argv_addr = start_addr;
                let mut current_argv_ptr = ARGV_START + USIZE_BYTES /* after argc */;
                // argv**
                for arg in argv {
                    copy_to_userspace(page_table, start_addr, &current_argv_ptr.to_ne_bytes());
                    start_addr += USIZE_BYTES;

                    current_argv_ptr += USIZE_BYTES; // skip the len
                    current_argv_ptr += arg.len() + 1; // skip the data
                }

                // set rdi and rsi to argc and argv
                // _start looks like: extern "C" _start(argc: u64, argv: *const (len, str))
                #[cfg(target_arch = "x86_64")]
                {
                    context.rdi = argc as u64;
                    context.rsi = argv_addr as u64;
                }
                // looks like this: argc: 8 (u64) -> argv: (len: 8 (u64) + bytes: len ([u8])) * argc -> argv_pointers: 8 (u64) * argc
                // where numbers is bytes count, (TYPE) is the type of the bytes
            }
        }

        #[cfg(target_arch = "x86_64")]
        {
            use arch::x86_64::threading::RFLAGS;

            context.rsp = STACK_END as u64;
            context.rip = function as u64;

            // Kernel process
            if flags.is_empty() {
                context.rflags = RFLAGS::from_bits_retain(0x202);

                context.ss = arch::x86_64::gdt::KERNEL_DATA_SEG as u64;
                context.cs = arch::x86_64::gdt::KERNEL_CODE_SEG as u64;
            } else if flags.contains(ProcessFlags::USERSPACE) {
                context.rflags = RFLAGS::IOPL_LOW
                    | RFLAGS::IOPL_HIGH
                    | RFLAGS::INTERRUPT_FLAG
                    | RFLAGS::from_bits_retain(0x2);

                context.ss = arch::x86_64::gdt::USER_DATA_SEG as u64;
                context.cs = arch::x86_64::gdt::USER_CODE_SEG as u64;
            }
            context.cr3 = root_page_table_addr as u64;
        }

        Ok(Process {
            ppid,
            pid,
            name,
            status,
            context,

            state: ProcessState::Alive(AliveProcessState::new(
                current_work_dir,
                root_page_table_addr,
                data_start,
            )),
        })
    }

    #[inline(always)]
    /// creates a userspace process from an elf, gives it pid 0 as a placeholder
    pub fn from_elf(
        owner_pid: usize,
        elf: Elf,
        name: &str,
        current_work_dir: String,
        argv: &[&str],
    ) -> Result<Self, ElfError> {
        let page_table_addr = paging::allocate_pml4().map_err(|_| ElfError::MapToError)?;

        let data_break =
            unsafe { elf.load_exec(&mut *((page_table_addr | hddm()) as *mut PageTable))? };

        let process = Self::new(
            elf.header.entry_point,
            owner_pid,
            0,
            name,
            argv,
            data_break,
            page_table_addr,
            current_work_dir,
            ProcessFlags::USERSPACE,
        )
        .ok()
        .ok_or(ElfError::MapToError)?;

        Ok(process)
    }

    /// makes a process a zombie
    /// does nothing if the process is already a zombie
    /// also moves the parentership of the process (it's children) to it's parent
    pub fn terminate(&mut self, exit_code: usize, terminator: usize) {
        if let ProcessState::Alive(ref mut state) = &mut self.state {
            let root_page_table = unsafe { &mut (*state.root_page_table) };
            unsafe { root_page_table.free(4) };

            let last_resource_id = state.resource_manager.lock().clean();
            let zombified = ProcessState::Zombie(ZombieProcessState {
                exit_code,
                exit_addr: self.context.at(),
                exit_stack_addr: self.context.stack_at(),
                killed_by: terminator,
                last_resource_id,
                data_start: state.data_start,
                data_break: state.data_break,
            });

            self.state = zombified;
            self.status = ProcessStatus::Zombie;
            // moves the parentership of all processes with `ppid` as `self.pid` to `self.ppid`
            // prevents orphan processes from being left behind
            // TODO: figure out if orphan processes should be killed
            super::for_each_where(|p| p.ppid == self.pid, |p| p.ppid = self.ppid);
            debug!(Process, "process with pid {} TERMINATED ...", self.pid);
        }
    }

    pub fn info(&self) -> ProcessInfo {
        let (
            exit_code,
            exit_addr,
            exit_stack_addr,
            killed_by,
            resource_count,
            data_start,
            data_break,
        ) = match &self.state {
            ProcessState::Zombie(state) => (
                state.exit_code,
                state.exit_addr,
                state.exit_stack_addr,
                state.killed_by,
                state.last_resource_id,
                state.data_start,
                state.data_break,
            ),
            ProcessState::Alive(state) => (
                0,
                0,
                0,
                0,
                state.resource_manager.lock().next_ri(),
                state.data_start,
                state.data_break,
            ),
        };

        ProcessInfo {
            ppid: self.ppid,
            pid: self.pid,
            name: self.name,
            status: self.status,

            exit_code,
            exit_addr,
            exit_stack_addr,

            killed_by,
            resource_count,
            data_start,
            data_break,
        }
    }
}
