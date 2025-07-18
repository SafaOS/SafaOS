use core::{
    arch::{asm, global_asm},
    cell::SyncUnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::AtomicUsize,
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use limine::mp::Cpu;
use safa_utils::abi::raw::processes::ContextPriority;

#[cfg(debug_assertions)]
use crate::sleep_until;
use crate::{
    PhysAddr, VirtAddr,
    arch::{
        aarch64::registers::MPIDR,
        disable_interrupts,
        paging::{CURRENT_HIGHER_HALF_TABLE, set_current_higher_page_table_phys},
    },
    debug,
    limine::MP_RESPONSE,
    memory::paging::{MapToError, PhysPageTable},
    threading::{
        self, CPULocalStorage, SCHEDULER_INITED,
        cpu_context::{self},
        process::Process,
    },
    utils::locks::Mutex,
};

use super::{
    exceptions::InterruptFrame,
    registers::{Reg, Spsr},
    timer,
};

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
    /// Creates a CPU Status Instance for Context (thread) 0
    /// Initializes a new userspace `CPUStatus` instance, initializes the stack, argv, etc...
    /// argument `userspace` determines if the process is in ring0 or not
    /// # Safety
    /// The caller must ensure `page_table` is not freed, as long as [`Self`] is alive otherwise it will cause UB
    pub unsafe fn create_root<const ARGS_COUNT: usize>(
        page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        entry_point_args: [usize; ARGS_COUNT],
        user_stack_end: VirtAddr,
        kernel_stack_end: VirtAddr,
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let entry_point = entry_point.into_raw() as u64;
        const { assert!(ARGS_COUNT <= 6) }

        let mut general_registers = [Reg::default(); 29];
        for (i, arg) in entry_point_args.iter().enumerate() {
            general_registers[i] = Reg(*arg as u64);
        }

        Ok(Self {
            sp_el0: user_stack_end,
            ttbr0: page_table.phys_addr(),
            frame: InterruptFrame {
                general_registers,
                sp: Reg(kernel_stack_end.into_raw() as u64),
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
        user_stack_end: VirtAddr,
        kernel_stack_end: VirtAddr,
        page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        context_id: cpu_context::Cid,
        arguments_ptr: *const (),
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let el0_stack_end = user_stack_end;
        let el1_stack_end = kernel_stack_end;

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
    if unsafe { *SCHEDULER_INITED.get() } {
        unsafe {
            let daif = super::get_daif();
            super::disable_interrupts();

            timer::TIMER_IRQ.set_pending();

            sleep_until!(10 ms, timer::TIMER_IRQ.is_pending());
            super::enable_interrupts();
            sleep_until!(10 ms, !timer::TIMER_IRQ.is_pending());

            super::set_daif(daif);
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

/// Creates a cpu local storage from a given process and an idle function
/// creates and adds a thread to the given process that is the idle thread for the caller CPU
///
/// unsafe because the caller is responsible for the memory which was allocated using a Box
unsafe fn create_cpu_local(
    process: &Arc<Process>,
    idle_function: fn() -> !,
) -> Result<(&'static CPULocalStorage, NonNull<CPUStatus>), MapToError> {
    let (thread, _) = Process::add_thread_to_process(
        process,
        VirtAddr::from(idle_function as usize),
        VirtAddr::null(),
        Some(ContextPriority::Low),
        None,
    )?;

    let status = unsafe { thread.context_unchecked().cpu_status() };

    let cpu_local_boxed = Box::new(CPULocalStorage::new(thread));

    unsafe {
        let cpu_local_ref = Box::into_non_null(cpu_local_boxed).as_ref();
        Ok((cpu_local_ref, status))
    }
}

unsafe fn add_new_cpu_local(
    process: &Arc<Process>,
    idle_function: fn() -> !,
) -> NonNull<CPUStatus> {
    let (cpu_local, status) = unsafe {
        create_cpu_local(process, idle_function).expect("failed to create a CPU local for a CPU")
    };
    unsafe {
        set_tpidr(VirtAddr::from_ptr(cpu_local));
    }
    CPU_LOCALS.lock().push(cpu_local);
    status
}

fn boot_core_inner(process: &Arc<Process>, idle_function: fn() -> !) -> ! {
    let cpuid = MPIDR::read().cpuid();
    unsafe {
        debug!("setting up CPU: {}", cpuid);

        let status = add_new_cpu_local(process, idle_function);
        let status = status.as_ref();

        debug!(
            "CPU {}: jumping to {:#x}, with stack at {:#x}",
            cpuid,
            status.at(),
            status.stack_at()
        );
        READY_CPUS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
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

        let (process, idle_function) = (*BOOT_CORE_ARGS.get()).assume_init_ref();
        boot_core_inner(process, *idle_function)
    }
}

static BOOT_CORE_ARGS: SyncUnsafeCell<MaybeUninit<(Arc<Process>, fn() -> !)>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());
pub(super) static READY_CPUS: AtomicUsize = AtomicUsize::new(1);

pub unsafe fn init_cpus(process: &Arc<Process>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let jmp_to = unsafe {
        // the current CPU should take local 0
        *BOOT_CORE_ARGS.get() = MaybeUninit::new((process.clone(), idle_function));
        add_new_cpu_local(process, idle_function)
    };

    let cpus = (*MP_RESPONSE).cpus();

    for cpu in cpus {
        if MPIDR::from_bits(cpu.mpidr).cpuid() != MPIDR::read().cpuid() {
            cpu.goto_address.write(boot_cpu);
        }
    }

    while READY_CPUS.load(core::sync::atomic::Ordering::Relaxed) != cpus.len() {
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
