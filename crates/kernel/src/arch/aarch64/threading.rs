use core::{
    arch::{asm, global_asm},
    cell::SyncUnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use limine::mp::Cpu;
use safa_utils::abi::raw::processes::{AbiStructures, ContextPriority};

use crate::{
    PhysAddr, VirtAddr,
    arch::{
        disable_interrupts,
        paging::{CURRENT_HIGHER_HALF_TABLE, set_current_higher_page_table_phys},
        registers::MPIDR,
    },
    debug,
    limine::MP_RESPONSE,
    memory::{
        copy_to_userspace, map_byte_slices, map_str_slices,
        paging::{EntryFlags, MapToError, PhysPageTable},
    },
    threading::{
        self, CPULocalStorage, SCHEDULER_INITED,
        cpu_context::{self},
        queue::ThreadQueue,
        task::Task,
    },
    utils::locks::Mutex,
};

use super::{
    exceptions::InterruptFrame,
    registers::{Reg, Spsr},
    timer,
};
use crate::memory::paging::PAGE_SIZE;

pub const STACK_SIZE: usize = PAGE_SIZE * 8;

pub const STACK0_START: VirtAddr = VirtAddr::from(0x00007A0000000000);
pub const STACK0_END: VirtAddr = STACK0_START + STACK_SIZE;

pub const GUARD_PAGE_COUNT: usize = 2;

pub const EL1_STACK_SIZE: usize = PAGE_SIZE * 8;
pub const EL1_STACK0_START: VirtAddr = VirtAddr::from(0x00007A1000000000);
pub const EL1_STACK0_END: VirtAddr = EL1_STACK0_START + EL1_STACK_SIZE;

pub const ENVIRONMENT_START: VirtAddr = VirtAddr::from(0x00007E0000000000);
pub const ARGV_START: VirtAddr = ENVIRONMENT_START + 0xA000000000;
pub const ENVIRONMENT_VARIABLES_START: VirtAddr = ENVIRONMENT_START + 0xE000000000;

pub const ABI_STRUCTURES_START: VirtAddr = ENVIRONMENT_START + 0x1000000000;

/// The CPU Status for each thread (registers)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CPUStatus {
    ttbr0: PhysAddr,
    sp_el0: VirtAddr,
    frame: InterruptFrame,
}

impl CPUStatus {
    fn new(frame: &mut InterruptFrame, page_table_addr: PhysAddr, sp_el0: VirtAddr) -> Self {
        Self {
            frame: *frame,
            ttbr0: page_table_addr,
            sp_el0,
        }
    }

    /// SHOULD ONLY BE CALLED FROM EL1
    unsafe fn from_current(frame: &mut InterruptFrame) -> Self {
        let ttbr0: usize;
        let sp_el0: usize;

        unsafe {
            asm!("mrs {}, sp_el0; mrs {}, ttbr0_el1", out(reg) sp_el0, out(reg) ttbr0);
        }

        Self::new(frame, PhysAddr::from(ttbr0), VirtAddr::from(sp_el0))
    }
}

global_asm!(
    "
.text
.global restore_cpu_status
.global restore_cpu_status_partial
restore_cpu_status_partial:
    ldp xzr, x2, [x0]
    msr sp_el0, x2

    mov x1, #0x10
    add x0, x0, x1
    b restore_frame

restore_cpu_status:
    ldp x1, x2, [x0]
    # x0 has to be a higher half address or everything breaks....
    # loads the translation table and the stack pointer
    msr ttbr0_el1, x1
    # reload address space
    tlbi VMALLE1
    dsb ISH
    isb

    msr sp_el0, x2

    mov x1, #0x10
    add x0, x0, x1
    b restore_frame
"
);

unsafe extern "C" {
    ///  Takes a reference to [`CPUStatus`] and sets current cpu status (registers) to it
    pub fn restore_cpu_status(status: &CPUStatus) -> !;
    fn restore_cpu_status_partial(status: &CPUStatus) -> !;
}

impl CPUStatus {
    /// Allocates a stack for a new thread, returns the end address of the stack
    unsafe fn allocate_stack_for_context(
        root_page_table: &mut PhysPageTable,
        context_id: cpu_context::Cid,
    ) -> Result<VirtAddr, MapToError> {
        let guard_pages_size = GUARD_PAGE_COUNT * PAGE_SIZE;

        let next_stack_start =
            STACK0_START + ((STACK_SIZE + guard_pages_size) * (context_id as usize));
        let next_stack_end = next_stack_start + STACK_SIZE;

        assert!(
            next_stack_start < EL1_STACK0_START,
            "there is no way you allocated 64 GiBs worth of thread stacks :skull:"
        );

        unsafe {
            root_page_table.alloc_map(
                next_stack_start,
                next_stack_end,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )?;
        }

        Ok(next_stack_end)
    }

    /// Allocates a stack for a new thread, returns the end address of the stack
    unsafe fn allocate_el1_stack_for_context(
        root_page_table: &mut PhysPageTable,
        context_id: cpu_context::Cid,
    ) -> Result<VirtAddr, MapToError> {
        let guard_pages_size = GUARD_PAGE_COUNT * PAGE_SIZE;

        let next_stack_start =
            EL1_STACK0_START + ((EL1_STACK_SIZE + guard_pages_size) * (context_id as usize));
        let next_stack_end = next_stack_start + STACK_SIZE;
        unsafe {
            root_page_table.alloc_map(
                next_stack_start,
                next_stack_end,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )?;
        }

        Ok(next_stack_end)
    }

    /// Creates a CPU Status Instance for Context (thread) 0
    /// Initializes a new userspace `CPUStatus` instance, initializes the stack, argv, etc...
    /// argument `userspace` determines if the process is in ring0 or not
    /// # Safety
    /// The caller must ensure `page_table` is not freed, as long as [`Self`] is alive otherwise it will cause UB
    pub unsafe fn create_root(
        page_table: &mut PhysPageTable,
        argv: &[&str],
        env: &[&[u8]],
        structures: AbiStructures,
        entry_point: VirtAddr,
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let entry_point = entry_point.into_raw() as u64;
        unsafe {
            // allocate the stack for thread 0
            page_table.alloc_map(
                STACK0_START,
                STACK0_END,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )?;

            page_table.alloc_map(
                EL1_STACK0_START,
                EL1_STACK0_END,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )?;
        }

        let argc = argv.len();
        let envc = env.len();

        let argv_ptr = map_str_slices(page_table, argv, ARGV_START)?;
        let argv_ptr = argv_ptr
            .map(|p| p.as_ptr())
            .unwrap_or(core::ptr::null_mut());

        let env_ptr = map_byte_slices(page_table, env, ENVIRONMENT_VARIABLES_START)?;
        let env_ptr = env_ptr.map(|p| p.as_ptr()).unwrap_or(core::ptr::null_mut());

        // ABI structures are structures that are passed to tasks by the kernel
        // currently only stdio is passed
        let structures_bytes: &[u8] =
            &unsafe { core::mem::transmute::<_, [u8; size_of::<AbiStructures>()]>(structures) };

        unsafe {
            page_table.alloc_map(
                ABI_STRUCTURES_START,
                ABI_STRUCTURES_START + PAGE_SIZE,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )?;
            copy_to_userspace(page_table, ABI_STRUCTURES_START, structures_bytes);
        }

        let abi_structures_ptr = ABI_STRUCTURES_START.into_ptr::<AbiStructures>();

        let mut general_registers = [Reg::default(); 29];
        general_registers[0] = Reg(argc as u64);
        general_registers[1] = Reg(argv_ptr as u64);
        general_registers[2] = Reg(envc as u64);
        general_registers[3] = Reg(env_ptr as u64);
        general_registers[4] = Reg(abi_structures_ptr as u64);

        Ok(Self {
            sp_el0: STACK0_END,
            ttbr0: page_table.phys_addr(),
            frame: InterruptFrame {
                general_registers,
                sp: Reg(EL1_STACK0_END.into_raw() as u64),
                elr: Reg(entry_point),
                lr: Reg(entry_point),
                spsr: if !userspace {
                    Spsr::EL1H
                } else {
                    Spsr::empty()
                },
                ..Default::default()
            },
        })
    }

    /// Creates a child CPU Status Instance, that is status of a thread child of thread 0
    pub unsafe fn create_child(
        page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        context_id: cpu_context::Cid,
        arguments_ptr: *const (),
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let el0_stack_end = unsafe { Self::allocate_stack_for_context(page_table, context_id)? };
        let el1_stack_end =
            unsafe { Self::allocate_el1_stack_for_context(page_table, context_id)? };

        let mut general_registers = [Reg::default(); 29];
        general_registers[0] = Reg(context_id as u64);
        general_registers[1] = Reg(arguments_ptr as u64);

        Ok(Self {
            ttbr0: page_table.phys_addr(),
            sp_el0: el0_stack_end,
            frame: InterruptFrame {
                general_registers,
                sp: Reg(el1_stack_end.into_raw() as u64),
                elr: Reg(entry_point.into_raw() as u64),
                lr: Reg(entry_point.into_raw() as u64),
                spsr: if !userspace {
                    Spsr::EL1H
                } else {
                    Spsr::empty()
                },
                ..Default::default()
            },
        })
    }

    pub fn at(&self) -> VirtAddr {
        VirtAddr::from(*self.frame.elr as usize)
    }

    pub fn stack_at(&self) -> VirtAddr {
        self.sp_el0
    }
}

pub(super) unsafe fn context_switch(frame: &mut InterruptFrame, before_switch: impl FnOnce()) {
    if crate::PANCIKED.load(core::sync::atomic::Ordering::Acquire) > 0 {
        crate::khalt()
    }

    let context = unsafe { CPUStatus::from_current(frame) };
    let swtch_results = threading::swtch(context);
    if let Some((new_context_ptr, address_space_changed)) = swtch_results {
        unsafe {
            before_switch();
            if !address_space_changed {
                restore_cpu_status_partial(new_context_ptr.as_ref());
            } else {
                restore_cpu_status(new_context_ptr.as_ref());
            }
        }
    } else {
        core::hint::cold_path();
        before_switch();
    }
}

pub fn invoke_context_switch() {
    if SCHEDULER_INITED.load(core::sync::atomic::Ordering::Acquire) {
        unsafe {
            super::disable_interrupts();
            timer::TIMER_IRQ.set_pending();
            debug_assert!(timer::TIMER_IRQ.is_pending());
            super::enable_interrupts();
            while timer::TIMER_IRQ.is_pending() {
                core::hint::spin_loop();
            }
            super::disable_interrupts();
        }
    }
}

static CPU_LOCALS: Mutex<Vec<&CPULocalStorage>> = Mutex::new(Vec::new());

unsafe fn set_tpidr(value: VirtAddr) {
    crate::serial!("tpidr_el1 set to: {value:#x}\n");
    unsafe {
        asm!("msr tpidr_el1, {}", in(reg) value.into_raw(), options(nomem, nostack));
    }
}

/// Creates a cpu local storage from a given task and an idle function
/// creates and adds a thread to the given task that is the idle thread for the caller CPU
///
/// unsafe because the caller is responsible for the memory which was allocated using a Box
unsafe fn create_cpu_local(
    task: &Arc<Task>,
    idle_function: fn() -> !,
) -> Result<(&'static CPULocalStorage, NonNull<CPUStatus>), MapToError> {
    let (thread, _) = Task::add_thread_to_task(
        task,
        VirtAddr::from(idle_function as usize),
        VirtAddr::null(),
        Some(ContextPriority::Low),
    )?;

    let status = unsafe { thread.context().cpu_status() };
    let mut queue = ThreadQueue::new();
    queue.push_back(thread);

    let cpu_local = CPULocalStorage::new(queue);
    let cpu_local_boxed = Box::new(cpu_local);

    unsafe {
        let cpu_local_ref = Box::into_non_null(cpu_local_boxed).as_ref();
        Ok((cpu_local_ref, status))
    }
}

unsafe fn add_new_cpu_local(task: &Arc<Task>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let (cpu_local, status) = unsafe {
        create_cpu_local(task, idle_function).expect("failed to create a CPU local for a CPU")
    };
    unsafe {
        set_tpidr(VirtAddr::from_ptr(cpu_local));
    }
    CPU_LOCALS.lock().push(cpu_local);
    status
}

fn boot_core_inner(task: &Arc<Task>, idle_function: fn() -> !) -> ! {
    let cpuid = MPIDR::read().cpuid();
    unsafe {
        debug!("setting up CPU: {}", cpuid);

        let status = add_new_cpu_local(task, idle_function);
        let status = status.as_ref();

        debug!(
            "CPU {}: jumping to {:#x}, with stack at {:#x}",
            cpuid,
            status.at(),
            status.stack_at()
        );

        restore_cpu_status(status)
    }
}

extern "C" fn boot_cpu(_: &Cpu) -> ! {
    unsafe {
        disable_interrupts();
    }
    super::setup_cpu_generic0();

    unsafe {
        let ttbr1_el1 = *CURRENT_HIGHER_HALF_TABLE.get();
        set_current_higher_page_table_phys(ttbr1_el1);
        super::setup_cpu_generic1();

        let (task, idle_function) = (*BOOT_CORE_ARGS.get()).assume_init_ref();
        boot_core_inner(task, *idle_function)
    }
}

static BOOT_CORE_ARGS: SyncUnsafeCell<MaybeUninit<(Arc<Task>, fn() -> !)>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());

pub unsafe fn init_cpus(task: &Arc<Task>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let jmp_to = unsafe {
        // the current CPU should take local 0
        *BOOT_CORE_ARGS.get() = MaybeUninit::new((task.clone(), idle_function));
        add_new_cpu_local(task, idle_function)
    };

    let cpus = (*MP_RESPONSE).cpus();

    for cpu in cpus {
        if MPIDR::from_bits(cpu.mpidr).cpuid() != MPIDR::read().cpuid() {
            cpu.goto_address.write(boot_cpu);
        }
    }

    while CPU_LOCALS.lock().len() != cpus.len() {
        core::hint::spin_loop();
    }

    jmp_to
}

/// Retrieves a pointer local to each CPU to a CPU Local Storage
pub fn cpu_local_storage_ptr() -> *mut CPULocalStorage {
    let ptr: *mut CPULocalStorage;
    unsafe { asm!("mrs {}, tpidr_el1", out(reg) ptr, options(nostack, nomem)) }
    ptr
}

/// Returns a list of pointers of CPU local storage to each cpu, can then be used by the scheduler to manage distrubting threads across CPUs
pub unsafe fn cpu_local_storages() -> &'static [&'static CPULocalStorage] {
    // only is called after the CPUs are initialized so should be safe
    unsafe { &*CPU_LOCALS.data_ptr() }
}
