pub const STACK_SIZE: usize = PAGE_SIZE * 8;

pub const STACK0_START: VirtAddr = VirtAddr::from(0x00007A3000000000);
pub const STACK0_END: VirtAddr = STACK0_START + STACK_SIZE;
pub const GUARD_PAGES_COUNT: usize = 1;

const RING0_STACK0_START: VirtAddr = VirtAddr::from(0x00007A0000000000);
const RING0_STACK0_END: VirtAddr = RING0_STACK0_START + STACK_SIZE;

pub const ENVIRONMENT_START: VirtAddr = VirtAddr::from(0x00007E0000000000);
pub const ARGV_START: VirtAddr = ENVIRONMENT_START + 0xA000000000;
pub const ENVIRONMENT_VARIABLES_START: VirtAddr = ENVIRONMENT_START + 0xE000000000;

pub const ABI_STRUCTURES_START: VirtAddr = ENVIRONMENT_START + 0x1000000000;
use crate::{
    PhysAddr,
    arch::{
        disable_interrupts,
        paging::{CURRENT_RING0_PAGE_TABLE, set_current_page_table_phys},
        x86_64::{
            gdt::{TSS0_PTR, TaskStateSegment, set_kernel_tss_stack},
            registers::wrmsr,
        },
    },
    debug,
    limine::MP_RESPONSE,
    memory::{map_byte_slices, map_str_slices},
    threading::{CPULocalStorage, cpu_context, task::Task},
    utils::locks::Mutex,
};
use core::{
    arch::{asm, global_asm},
    cell::SyncUnsafeCell,
    mem::{MaybeUninit, offset_of},
    ptr::NonNull,
    sync::atomic::AtomicUsize,
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use bitflags::bitflags;
use limine::mp::Cpu;

use crate::{
    VirtAddr,
    memory::{
        copy_to_userspace,
        paging::{EntryFlags, MapToError, PAGE_SIZE, PhysPageTable},
    },
    threading::swtch,
};

use super::gdt::{KERNEL_CODE_SEG, KERNEL_DATA_SEG, USER_CODE_SEG, USER_DATA_SEG};

bitflags! {
    #[derive(Default, Debug, Clone, Copy)]
    #[repr(C)]
    pub struct RFLAGS: u64 {
        const ID = 1 << 21;
        const VIRTUAL_INTERRUPT_PENDING = 1 << 20;
        const VIRTUAL_INTERRUPT = 1 << 19;
        const ALIGNMENT_CHECK = 1 << 18;
        const VIRTUAL_8086_MODE = 1 << 17;

        const RESUME_FLAG = 1 << 16;
        const NESTED_TASK = 1 << 14;

        const IOPL_HIGH = 1 << 13;
        const IOPL_LOW = 1 << 12;

        const OVERFLOW_FLAG = 1 << 11;
        const DIRECTION_FLAG = 1 << 10;

        const INTERRUPT_FLAG = 1 << 9;
        const TRAP_FLAG = 1 << 8;

        const SIGN_FLAG = 1 << 7;
        const ZERO_FLAG = 1 << 6;
        const AUXILIARY_CARRY_FLAG = 1 << 4;

        const PARITY_FLAG = 1 << 2;
        const CARRY_FLAG = 1;
    }
}

/// The CPU Status for each thread (registers)
#[derive(Debug, Clone, Copy, Default)]
#[repr(C, packed)]
pub struct CPUStatus {
    ring0_rsp: VirtAddr,
    rsp: VirtAddr,
    rflags: RFLAGS,
    ss: u64,
    cs: u64,

    rip: VirtAddr,

    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,

    rbp: u64,
    rdi: u64,
    rsi: u64,

    rdx: u64,
    rcx: u64,
    rbx: u64,
    cr3: PhysAddr,
    rax: u64,

    // ffi-safe alternative for u128
    xmm15: [u8; 16],
    xmm14: [u8; 16],
    xmm13: [u8; 16],
    xmm12: [u8; 16],
    xmm11: [u8; 16],
    xmm10: [u8; 16],
    xmm9: [u8; 16],
    xmm8: [u8; 16],
    xmm7: [u8; 16],
    xmm6: [u8; 16],
    xmm5: [u8; 16],
    xmm4: [u8; 16],
    xmm3: [u8; 16],
    xmm2: [u8; 16],
    xmm1: [u8; 16],
    xmm0: [u8; 16],
}

use safa_utils::abi::raw::processes::{AbiStructures, ContextPriority};

const fn make_usermode_regs(is_userspace: bool) -> (u64, u64, RFLAGS) {
    if is_userspace {
        (
            USER_CODE_SEG as u64,
            USER_DATA_SEG as u64,
            RFLAGS::IOPL_LOW
                .union(RFLAGS::IOPL_HIGH)
                .union(RFLAGS::from_bits_retain(0x202)),
        )
    } else {
        (
            KERNEL_CODE_SEG as u64,
            KERNEL_DATA_SEG as u64,
            RFLAGS::from_bits_retain(0x202),
        )
    }
}

unsafe fn allocate_generic_stack_for_context(
    stack_generic_start: VirtAddr,
    page_table: &mut PhysPageTable,
    context_id: cpu_context::Cid,
) -> Result<VirtAddr, MapToError> {
    let guard_pages_size = GUARD_PAGES_COUNT * PAGE_SIZE;
    let stack_start = stack_generic_start + (context_id as usize * (STACK_SIZE + guard_pages_size));
    let stack_end = stack_start + STACK_SIZE;

    unsafe {
        page_table.alloc_map(
            stack_start,
            stack_end,
            EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
        )?;
    }

    Ok(stack_end)
}

unsafe fn allocate_user_stack_for_context(
    page_table: &mut PhysPageTable,
    context_id: cpu_context::Cid,
) -> Result<VirtAddr, MapToError> {
    unsafe { allocate_generic_stack_for_context(STACK0_START, page_table, context_id) }
}

unsafe fn allocate_kernel_stack_for_context(
    page_table: &mut PhysPageTable,
    context_id: cpu_context::Cid,
) -> Result<VirtAddr, MapToError> {
    unsafe { allocate_generic_stack_for_context(RING0_STACK0_START, page_table, context_id) }
}

impl CPUStatus {
    pub fn at(&self) -> VirtAddr {
        self.rip
    }

    pub fn stack_at(&self) -> VirtAddr {
        self.rsp
    }

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
        unsafe {
            // allocate the stack
            page_table.alloc_map(
                STACK0_START,
                STACK0_END,
                EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
            )?;

            // allocate the syscall stack
            page_table.alloc_map(
                RING0_STACK0_START,
                RING0_STACK0_END,
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
            copy_to_userspace(page_table, ABI_STRUCTURES_START.into(), structures_bytes);
        }

        let abi_structures_ptr = ABI_STRUCTURES_START.into_ptr::<AbiStructures>();

        let (cs, ss, rflags) = make_usermode_regs(userspace);

        Ok(Self {
            ring0_rsp: RING0_STACK0_END,
            rflags,
            rip: entry_point,
            rdi: argc as u64,
            rsi: argv_ptr as u64,
            rdx: envc as u64,
            rcx: env_ptr as u64,
            r8: abi_structures_ptr as u64,
            cr3: page_table.phys_addr(),
            rsp: STACK0_END,
            cs,
            ss,
            ..Default::default()
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
        unsafe {
            let user_stack_end = allocate_user_stack_for_context(page_table, context_id)?;
            let kernel_stack_end = allocate_kernel_stack_for_context(page_table, context_id)?;

            let (cs, ss, rflags) = make_usermode_regs(userspace);

            Ok(Self {
                ring0_rsp: kernel_stack_end,
                rflags,
                rip: entry_point,
                rdi: context_id as u64,
                rsi: arguments_ptr as u64,
                cr3: page_table.phys_addr(),
                rsp: user_stack_end,
                cs,
                ss,
                ..Default::default()
            })
        }
    }
}

global_asm!(include_str!("./threading.asm"), default_kernel_stack = const RING0_STACK0_END.into_raw());

unsafe extern "C" {
    /// Takes a reference to [`CPUStatus`] and sets current cpu status (registers) to it
    /// also reloads the address space
    /// assumes that the `status` is valid and points to a valid [`CPUStatus`] structure that is accessible by the new address space
    pub fn restore_cpu_status_full(status: *const CPUStatus) -> !;
    /// same as [`restore_cpu_status_full`] but does not reload the address space
    pub fn restore_cpu_status_partial(status: *const CPUStatus) -> !;
}

unsafe extern "x86-interrupt" {
    pub fn context_switch_stub();
}

#[unsafe(no_mangle)]
pub extern "C" fn context_switch(
    mut capture: CPUStatus,
    frame: super::interrupts::InterruptFrame,
) -> ! {
    capture.rsp = frame.stack_pointer;
    capture.rip = frame.insturaction;

    capture.cs = frame.code_segment;
    capture.ss = frame.stack_segment;
    capture.rflags = frame.flags;

    unsafe {
        let swtch_results = swtch(capture);

        super::interrupts::apic::send_eoi();
        if let Some((new_context_ptr, address_space_changed)) = swtch_results {
            let new_context_ref = new_context_ptr.as_ref();

            set_kernel_tss_stack(new_context_ref.ring0_rsp);
            if address_space_changed {
                capture = *new_context_ref;
                restore_cpu_status_full(&capture);
            } else {
                restore_cpu_status_partial(new_context_ref);
            }
        } else {
            core::hint::cold_path();
            restore_cpu_status_partial(&capture);
        }
    }
}

#[inline(always)]
pub fn invoke_context_switch() {
    unsafe { asm!("int 0x20") }
}

/// Fully restores the CPU status from the given [`CPUStatus`] structure.
/// shouldn't be used
pub unsafe fn restore_cpu_status(status: *const CPUStatus) -> ! {
    unsafe {
        restore_cpu_status_full(status);
    }
}

static CPU_LOCALS: Mutex<Vec<&ArchCPULocalStorage>> = Mutex::new(Vec::new());
static BOOT_CORE_ARGS: SyncUnsafeCell<MaybeUninit<(Arc<Task>, fn() -> !)>> =
    SyncUnsafeCell::new(MaybeUninit::uninit());
static READY_CPUS: AtomicUsize = AtomicUsize::new(1);

unsafe fn set_gs(value: VirtAddr) {
    crate::serial!("gs set to: {value:#x} + 8\n");

    unsafe {
        wrmsr(0xC0000101, value.into_raw() as u64);
        wrmsr(0xC0000102, value.into_raw() as u64);
        asm!("swapgs");
    }
}

/// Creates a cpu local storage from a given task and an idle function
/// creates and adds a thread to the given task that is the idle thread for the caller CPU
///
/// unsafe because the caller is responsible for the memory which was allocated using a Box
unsafe fn create_cpu_local(
    tss_ptr: *mut TaskStateSegment,
    task: &Arc<Task>,
    idle_function: fn() -> !,
) -> Result<(&'static ArchCPULocalStorage, NonNull<CPUStatus>), MapToError> {
    assert!(!tss_ptr.is_null());

    let (thread, _) = Task::add_thread_to_task(
        task,
        VirtAddr::from(idle_function as usize),
        VirtAddr::null(),
        Some(ContextPriority::Low),
    )?;

    let status = unsafe { thread.context().cpu_status() };

    let cpu_local = CPULocalStorage::new(thread);
    let arch_cpu_local_boxed = Box::new(ArchCPULocalStorage {
        cpu_local,
        tss_ptr,
        ptr_to_self: core::ptr::dangling(),
    });

    let cpu_local_ref = Box::leak(arch_cpu_local_boxed);
    (*cpu_local_ref).ptr_to_self = cpu_local_ref;
    Ok((&*cpu_local_ref, status))
}

unsafe fn add_new_cpu_local(
    tss_ptr: *mut TaskStateSegment,
    task: &Arc<Task>,
    idle_function: fn() -> !,
) -> NonNull<CPUStatus> {
    let (cpu_local, status) = unsafe {
        create_cpu_local(tss_ptr, task, idle_function)
            .expect("failed to create a CPU local for a CPU")
    };
    unsafe {
        set_gs(
            VirtAddr::from_ptr(cpu_local as *const ArchCPULocalStorage)
                + offset_of!(ArchCPULocalStorage, ptr_to_self),
        );
    }
    CPU_LOCALS.lock().push(cpu_local);
    status
}

fn boot_core_inner(
    tss_ptr: *mut TaskStateSegment,
    lapic_id: u8,
    task: &Arc<Task>,
    idle_function: fn() -> !,
) -> ! {
    unsafe {
        debug!("setting up CPU with lapic ID: {lapic_id}");

        let status = add_new_cpu_local(tss_ptr, task, idle_function);
        let status = status.as_ref();

        debug!(
            "CPU with lapic ID {}: jumping to {:#x}, with stack at {:#x}",
            lapic_id,
            status.at(),
            status.stack_at()
        );
        READY_CPUS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        restore_cpu_status(status)
    }
}

extern "C" fn boot_cpu(cpu: &Cpu) -> ! {
    unsafe {
        disable_interrupts();
    }
    let tss_ptr = super::setup_cpu_generic0();

    unsafe {
        let phys_addr = *CURRENT_RING0_PAGE_TABLE.get();
        set_current_page_table_phys(phys_addr);

        crate::serial!("waaah: {}\n", cpu.lapic_id);
        // FIXME: calibrate each CPU's TSC
        let mut _ignored = 0;
        super::setup_cpu_generic1(&mut _ignored);

        let (task, idle_function) = (*BOOT_CORE_ARGS.get()).assume_init_ref();
        boot_core_inner(tss_ptr, cpu.lapic_id as u8, task, *idle_function)
    }
}

pub unsafe fn init_cpus(task: &Arc<Task>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let jmp_to = unsafe {
        // the current CPU should take local 0
        *BOOT_CORE_ARGS.get() = MaybeUninit::new((task.clone(), idle_function));
        add_new_cpu_local(*TSS0_PTR as *mut TaskStateSegment, task, idle_function)
    };

    let cpus = (*MP_RESPONSE).cpus();

    for cpu in &cpus[1..] {
        crate::serial!("cccpu: {}\n", cpu.lapic_id);
        cpu.goto_address.write(boot_cpu);
    }

    while READY_CPUS.load(core::sync::atomic::Ordering::Relaxed) != cpus.len() {
        core::hint::spin_loop();
    }

    jmp_to
}

#[repr(C)]
pub(in crate::arch::x86_64) struct ArchCPULocalStorage {
    cpu_local: CPULocalStorage,
    pub tss_ptr: *mut TaskStateSegment,
    ptr_to_self: *const Self,
}

unsafe impl Send for ArchCPULocalStorage {}
unsafe impl Sync for ArchCPULocalStorage {}

pub(in crate::arch::x86_64) fn arch_cpu_local_storage_ptr() -> *mut ArchCPULocalStorage {
    let ptr: *mut ArchCPULocalStorage;
    unsafe { asm!("mov {}, gs:0", out(reg) ptr) }
    ptr
}
/// Retrieves a pointer local to each CPU to a CPU Local Storage
pub fn cpu_local_storage_ptr() -> *mut CPULocalStorage {
    arch_cpu_local_storage_ptr().cast()
}

/// Returns a list of pointers of CPU local storage to each cpu, can then be used by the scheduler to manage distrubting threads across CPUs
pub unsafe fn cpu_local_storages() -> &'static [&'static CPULocalStorage] {
    // only is called after the CPUs are initialized so should be safe
    unsafe {
        &*((&*CPU_LOCALS.data_ptr()).as_slice() as *const [&ArchCPULocalStorage]
            as *const [&CPULocalStorage])
    }
}
