pub const STACK_SIZE: usize = PAGE_SIZE * 8;

use crate::{
    PhysAddr,
    arch::x86_64::{
        gdt::{get_kernel_tss_stack, set_kernel_tss_stack},
        interrupts::handlers::InterruptCpuFrame,
        registers::{RFLAGS, rdmsr, wrmsr},
    },
    globals::KERNEL_ELF,
    thread::Tid,
};
use core::{arch::global_asm, fmt::Display};

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

impl Display for CPUStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Saved general purpose registers:")?;
        macro_rules! reg {
            ($name:ident) => {
                write!(f, "{:<3}: {:#018x}    ", stringify!($name), self.$name)?;
            };
        }

        reg!(rax);
        reg!(rbx);
        reg!(rcx);

        writeln!(f)?;

        reg!(rdx);
        reg!(rdi);
        reg!(rsi);

        writeln!(f)?;

        reg!(r8);
        reg!(r9);
        reg!(r10);

        writeln!(f)?;
        reg!(r11);
        reg!(r12);
        reg!(r13);

        writeln!(f)?;
        reg!(r14);
        reg!(r15);

        write!(f, "\n\n")?;
        reg!(rsp);
        writeln!(f)?;
        reg!(ring0_rsp);
        writeln!(f)?;
        reg!(rbp);
        writeln!(f)?;
        reg!(cr3);
        writeln!(f)?;

        let sym = KERNEL_ELF.sym_from_value_range(self.rip);

        let name = sym.and_then(|sym| KERNEL_ELF.string_table_index(sym.name_index));
        let name = name.as_deref().unwrap_or("???");
        writeln!(f, "at {:?} <{}>", self.rip, name)?;
        writeln!(f, "rflags: {:#?}", self.rflags)?;
        Ok(())
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

#[unsafe(no_mangle)]
pub(super) extern "C" fn context_switch_on_int(switch_frame: &mut InterruptCpuFrame) {
    let capture = &mut switch_frame.capture;

    unsafe {
        context_switch_and_return_inner(
            capture,
            || {
                super::interrupts::apic::send_eoi();
            },
            false,
        )
    }
}

#[inline(always)]
unsafe fn context_switch_and_return_inner(
    capture: &mut CPUStatus,
    before_switch: impl FnOnce(),
    is_thread_yielding: bool,
) {
    let Err(before_switch) = {
        unsafe {
            capture.ring0_rsp = get_kernel_tss_stack();
            capture.fs_base = VirtAddr::from(rdmsr(0xC0000100));
        }
        swtch(capture, before_switch, is_thread_yielding)
    };

    core::hint::cold_path();
    before_switch();
}

#[unsafe(no_mangle)]
extern "C" fn context_switch_and_return(capture: &mut CPUStatus) -> ! {
    unsafe { context_switch_and_return_inner(capture, || {}, true) }
    unsafe { restore_cpu_status_partial(capture) }
}

unsafe extern "C" {
    // TODO: please remember to use
    fn thread_yield_wrapper();
}

#[inline(never)]
pub fn invoke_context_switch() {
    unsafe { thread_yield_wrapper() }
}

/// Fully restores the CPU status from the given [`CPUStatus`] structure.
/// shouldn't be used
pub unsafe fn restore_cpu_status(status: &CPUStatus) -> ! {
    unsafe { restore_cpu_status_full_all(status) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn restore_cpu_status_partial_all(status: &CPUStatus) -> ! {
    unsafe {
        set_kernel_tss_stack(status.ring0_rsp);
        wrmsr(0xC0000100, status.fs_base.into_raw() as u64);
        restore_cpu_status_partial(status)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn restore_cpu_status_full_all(status: &CPUStatus) -> ! {
    unsafe {
        set_kernel_tss_stack(status.ring0_rsp);
        wrmsr(0xC0000100, status.fs_base.into_raw() as u64);
        restore_cpu_status_full(status)
    }
}
