pub const STACK_SIZE: usize = PAGE_SIZE * 8;

use crate::{
    PhysAddr,
    arch::{
        paging::{CURRENT_RING0_PAGE_TABLE, set_current_page_table_phys},
        without_interrupts,
        x86_64::{
            gdt::{TSS0_PTR, TaskStateSegment, get_kernel_tss_stack, set_kernel_tss_stack},
            registers::{RFLAGS, rdmsr, wrmsr},
        },
    },
    debug,
    limine::MP_RESPONSE,
    process::Process,
    scheduler::{SCHEDULER_INITED, Scheduler},
    thread::Tid,
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
use limine::mp::Cpu;

use crate::{
    VirtAddr,
    memory::paging::{MapToError, PAGE_SIZE, PhysPageTable},
    scheduler::swtch,
};

use super::gdt::{KERNEL_CODE_SEG, KERNEL_DATA_SEG, USER_CODE_SEG, USER_DATA_SEG};

/// The CPU Status for each thread (registers)
#[derive(Debug, Clone, Copy)]
#[repr(C, align(16))]
pub struct CPUStatus {
    fs_base: VirtAddr,
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

    __: u64,
    floating_point: [u8; 512],
}

lazy_static::lazy_static! {
    static ref DEFAULT_CPU_STATUS: CPUStatus = {
        let mut results: CPUStatus = unsafe { core::mem::zeroed() };
             unsafe {
                 /* HACK to load correct mxcsr */
                 assert!(((&raw mut results.floating_point) as usize).is_multiple_of(16));
                 core::arch::asm!("fxsave [{}]", in(reg) &raw mut results.floating_point);
             }
             results
    };
}

use crate::thread::ContextPriority;

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
    pub unsafe fn create_root<const ARGS_COUNT: usize>(
        page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        entry_point_args: [usize; ARGS_COUNT],
        tls_addr: VirtAddr,
        user_stack_end: VirtAddr,
        kernel_stack_end: VirtAddr,
        userspace: bool,
    ) -> Result<Self, MapToError> {
        const { assert!(ARGS_COUNT <= 6) }

        let (cs, ss, rflags) = make_usermode_regs(userspace);

        macro_rules! entry_point_arg {
            ($index: literal) => {
                entry_point_args.get($index).copied().unwrap_or(0) as u64
            };
        }

        Ok(Self {
            fs_base: tls_addr,
            ring0_rsp: kernel_stack_end,
            rflags,
            rip: entry_point,
            rdi: entry_point_arg!(0),
            rsi: entry_point_arg!(1),
            rdx: entry_point_arg!(2),
            rcx: entry_point_arg!(3),
            r8: entry_point_arg!(4),
            r9: entry_point_arg!(5),
            cr3: page_table.phys_addr(),
            rsp: user_stack_end,
            cs,
            ss,
            ..*DEFAULT_CPU_STATUS
        })
    }

    /// Creates a child CPU Status Instance, that is status of a thread child of thread 0
    pub unsafe fn create_child(
        tls_addr: VirtAddr,
        user_stack_end: VirtAddr,
        kernel_stack_end: VirtAddr,
        page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        thread_id: Tid,
        arguments_ptr: *const (),
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let (cs, ss, rflags) = make_usermode_regs(userspace);

        Ok(Self {
            fs_base: tls_addr,
            ring0_rsp: kernel_stack_end,
            rflags,
            rip: entry_point,
            rdi: thread_id as u64,
            rsi: arguments_ptr as u64,
            cr3: page_table.phys_addr(),
            rsp: user_stack_end,
            cs,
            ss,
            ..*DEFAULT_CPU_STATUS
        })
    }
}

global_asm!(include_str!("./threading.asm"));

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

#[repr(C)]
struct ContextSwitchFrame {
    capture: CPUStatus,
    __: u64,
    int: super::interrupts::InterruptFrame,
}

#[unsafe(no_mangle)]
extern "C" fn context_switch(switch_frame: ContextSwitchFrame) -> ! {
    let mut capture = switch_frame.capture;
    let frame = switch_frame.int;

    capture.fs_base = VirtAddr::from(rdmsr(0xC0000100));
    capture.ring0_rsp = if unsafe { *SCHEDULER_INITED.get() } {
        unsafe { get_kernel_tss_stack() }
    } else {
        VirtAddr::null()
    };

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
            wrmsr(0xC0000100, new_context_ref.fs_base.into_raw() as u64);

            if address_space_changed {
                restore_cpu_status_full(new_context_ref);
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
pub unsafe fn restore_cpu_status(status: &CPUStatus) -> ! {
    unsafe {
        set_kernel_tss_stack(status.ring0_rsp);
        wrmsr(0xC0000100, status.fs_base.into_raw() as u64);
        restore_cpu_status_full(status);
    }
}

static CPU_LOCALS: Mutex<Vec<&ArchCPULocalStorage>> = Mutex::new(Vec::new());
static BOOT_CORE_ARGS: SyncUnsafeCell<MaybeUninit<(Arc<Process>, fn() -> !)>> =
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

/// Creates a cpu local storage from a given process and an idle function
/// creates and adds a thread to the given process that is the idle thread for the caller CPU
///
/// unsafe because the caller is responsible for the memory which was allocated using a Box
unsafe fn create_cpu_local(
    tss_ptr: *mut TaskStateSegment,
    process: &Arc<Process>,
    idle_function: fn() -> !,
) -> Result<(&'static ArchCPULocalStorage, NonNull<CPUStatus>), MapToError> {
    assert!(!tss_ptr.is_null());

    let (thread, _) = Process::new_thread(
        process,
        VirtAddr::from(idle_function as usize),
        VirtAddr::null(),
        Some(ContextPriority::Low),
        None,
    )?;

    let status = unsafe { thread.context_unchecked().cpu_status() };

    let cpu_local = Scheduler::new(thread);
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
    process: &Arc<Process>,
    idle_function: fn() -> !,
) -> NonNull<CPUStatus> {
    let (cpu_local, status) = unsafe {
        create_cpu_local(tss_ptr, process, idle_function)
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
    process: &Arc<Process>,
    idle_function: fn() -> !,
) -> ! {
    unsafe {
        debug!("setting up CPU with lapic ID: {lapic_id}");

        let status = add_new_cpu_local(tss_ptr, process, idle_function);
        let status_ref = status.as_ref();

        debug!(
            "CPU with lapic ID {}: jumping to {:#x}, with stack at {:#x}",
            lapic_id,
            status_ref.at(),
            status_ref.stack_at()
        );
        READY_CPUS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        restore_cpu_status(status_ref)
    }
}

extern "C" fn boot_cpu(cpu: &Cpu) -> ! {
    without_interrupts(|| {
        let tss_ptr = super::setup_cpu_generic0();

        unsafe {
            let phys_addr = *CURRENT_RING0_PAGE_TABLE.get();
            set_current_page_table_phys(phys_addr);

            // FIXME: calibrate each CPU's TSC
            let mut _ignored = 0;
            super::setup_cpu_generic1(&mut _ignored);

            let (process, idle_function) = (*BOOT_CORE_ARGS.get()).assume_init_ref();
            boot_core_inner(tss_ptr, cpu.lapic_id as u8, process, *idle_function)
        }
    })
}

pub unsafe fn init_cpus(process: &Arc<Process>, idle_function: fn() -> !) -> NonNull<CPUStatus> {
    let jmp_to = unsafe {
        // the current CPU should take local 0
        *BOOT_CORE_ARGS.get() = MaybeUninit::new((process.clone(), idle_function));
        add_new_cpu_local(*TSS0_PTR as *mut TaskStateSegment, process, idle_function)
    };

    let cpus = (*MP_RESPONSE).cpus();

    for cpu in &cpus[1..] {
        cpu.goto_address.write(boot_cpu);
    }

    while READY_CPUS.load(core::sync::atomic::Ordering::Relaxed) != cpus.len() {
        core::hint::spin_loop();
    }

    jmp_to
}

#[repr(C)]
pub(in crate::arch::x86_64) struct ArchCPULocalStorage {
    cpu_local: Scheduler,
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
pub fn cpu_local_storage_ptr() -> *mut Scheduler {
    arch_cpu_local_storage_ptr().cast()
}

/// Returns a list of pointers of CPU local storage to each cpu, can then be used by the scheduler to manage distrubting threads across CPUs
pub unsafe fn cpu_local_storages() -> &'static [&'static Scheduler] {
    // only is called after the CPUs are initialized so should be safe
    unsafe {
        &*((&*CPU_LOCALS.data_ptr()).as_slice() as *const [&ArchCPULocalStorage]
            as *const [&Scheduler])
    }
}
