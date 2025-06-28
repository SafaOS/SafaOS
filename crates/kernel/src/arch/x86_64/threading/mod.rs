pub const STACK_SIZE: usize = PAGE_SIZE * 8;
pub const STACK_START: VirtAddr = VirtAddr::from(0x00007A3000000000);
pub const STACK_END: VirtAddr = STACK_START + STACK_SIZE;

pub const RING0_STACK_START: VirtAddr = VirtAddr::from(0x00007A0000000000);
pub const RING0_STACK_END: VirtAddr = RING0_STACK_START + STACK_SIZE;

pub const ENVIRONMENT_START: VirtAddr = VirtAddr::from(0x00007E0000000000);
pub const ARGV_START: VirtAddr = ENVIRONMENT_START + 0xA000000000;
pub const ENVIRONMENT_VARIABLES_START: VirtAddr = ENVIRONMENT_START + 0xE000000000;

pub const ABI_STRUCTURES_START: VirtAddr = ENVIRONMENT_START + 0x1000000000;
use crate::memory::{map_byte_slices, map_str_slices};
use core::arch::{asm, global_asm};

use bitflags::bitflags;

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
    cr3: u64,
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

use safa_utils::abi::raw::processes::AbiStructures;

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
    pub unsafe fn create(
        page_table: &mut PhysPageTable,
        argv: &[&str],
        env: &[&[u8]],
        structures: AbiStructures,
        entry_point: VirtAddr,
        userspace: bool,
    ) -> Result<Self, MapToError> {
        // allocate the stack
        page_table.alloc_map(
            STACK_START,
            STACK_END,
            EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
        )?;

        // allocate the syscall stack
        page_table.alloc_map(
            RING0_STACK_START,
            RING0_STACK_END,
            EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
        )?;

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

        page_table.alloc_map(
            ABI_STRUCTURES_START,
            ABI_STRUCTURES_START + PAGE_SIZE,
            EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
        )?;
        copy_to_userspace(page_table, ABI_STRUCTURES_START.into(), structures_bytes);

        let abi_structures_ptr = ABI_STRUCTURES_START.into_ptr::<AbiStructures>();

        let (cs, ss, rflags) = if userspace {
            (
                USER_CODE_SEG as u64,
                USER_DATA_SEG as u64,
                RFLAGS::IOPL_LOW | RFLAGS::IOPL_HIGH | RFLAGS::from_bits_retain(0x202),
            )
        } else {
            (
                KERNEL_CODE_SEG as u64,
                KERNEL_DATA_SEG as u64,
                RFLAGS::from_bits_retain(0x202),
            )
        };

        Ok(Self {
            rflags,
            rip: entry_point,
            rdi: argc as u64,
            rsi: argv_ptr as u64,
            rdx: envc as u64,
            rcx: env_ptr as u64,
            r8: abi_structures_ptr as u64,
            cr3: page_table.phys_addr().into_raw() as u64,
            rsp: STACK_END,
            cs,
            ss,
            ..Default::default()
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
