use core::{
    arch::{asm, global_asm},
    cell::SyncUnsafeCell,
};

use safa_utils::abi::raw::processes::AbiStructures;

use crate::{
    arch::aarch64::{gic, timer},
    memory::{
        copy_to_userspace, map_byte_slices, map_str_slices,
        paging::{EntryFlags, MapToError, PhysPageTable},
    },
    threading, PhysAddr, VirtAddr,
};

use super::{
    exceptions::InterruptFrame,
    registers::{Reg, Spsr},
};
use crate::memory::paging::PAGE_SIZE;

/// Store the context to switch to in the higher half, so that it isn't affected by lower half translation table switch
static CURRENT_CONTEXT: SyncUnsafeCell<CPUStatus> =
    SyncUnsafeCell::new(unsafe { core::mem::zeroed() });

pub const STACK_SIZE: usize = PAGE_SIZE * 8;
pub const STACK_START: usize = 0x00007A0000000000;
pub const STACK_END: usize = STACK_START + STACK_SIZE;

pub const EL1_STACK_SIZE: usize = PAGE_SIZE * 8;
pub const EL1_STACK_START: usize = 0x00007A1000000000;
pub const EL1_STACK_END: usize = EL1_STACK_START + EL1_STACK_SIZE;

pub const ENVIRONMENT_START: usize = 0x00007E0000000000;
pub const ARGV_START: usize = ENVIRONMENT_START + 0xA000000000;
pub const ENVIRONMENT_VARIABLES_START: usize = ENVIRONMENT_START + 0xE000000000;

pub const ABI_STRUCTURES_START: usize = ENVIRONMENT_START + 0x1000000000;

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
        let ttbr0: PhysAddr;
        let sp_el0: VirtAddr;

        unsafe {
            asm!("mrs {}, sp_el0; mrs {}, ttbr0_el1", out(reg) sp_el0, out(reg) ttbr0);
        }

        Self::new(frame, ttbr0, sp_el0)
    }
}

global_asm!(
    "
.text
.global restore_cpu_status
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

extern "C" {
    ///  Takes a reference to [`CPUStatus`] and sets current cpu status (registers) to it
    pub fn restore_cpu_status(status: &CPUStatus) -> !;
}

impl CPUStatus {
    /// Initializes a new userspace `CPUStatus` instance, initializes the stack, argv, etc...
    /// argument `userspace` determines if the process is in ring0 or not
    /// # Safety
    /// The caller must ensure `page_table` is not freed, as long as [`Self`] is alive otherwise it will cause UB
    pub unsafe fn create(
        page_table: &mut PhysPageTable,
        argv: &[&str],
        env: &[&[u8]],
        structures: AbiStructures,
        entry_point: usize,
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let entry_point = entry_point as u64;
        // allocate the stack
        page_table.alloc_map(
            STACK_START,
            STACK_END,
            EntryFlags::WRITE | EntryFlags::USER_ACCESSIBLE,
        )?;

        page_table.alloc_map(
            EL1_STACK_START,
            EL1_STACK_END,
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
        copy_to_userspace(page_table, ABI_STRUCTURES_START, structures_bytes);

        let abi_structures_ptr = ABI_STRUCTURES_START as *const AbiStructures;

        let mut general_registers = [Reg::default(); 29];
        general_registers[0] = Reg(argc as u64);
        general_registers[1] = Reg(argv_ptr as u64);
        general_registers[2] = Reg(envc as u64);
        general_registers[3] = Reg(env_ptr as u64);
        general_registers[4] = Reg(abi_structures_ptr as u64);

        Ok(Self {
            sp_el0: STACK_END,
            ttbr0: page_table.phys_addr(),
            frame: InterruptFrame {
                general_registers,
                sp: Reg(EL1_STACK_END as u64),
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

    pub fn at(&self) -> VirtAddr {
        *self.frame.elr as VirtAddr
    }

    pub fn stack_at(&self) -> VirtAddr {
        self.sp_el0
    }
}

pub(super) unsafe fn context_switch(frame: &mut InterruptFrame, before_switch: impl FnOnce()) -> ! {
    let context = unsafe { CPUStatus::from_current(frame) };
    let new_context = threading::swtch(context);

    let current_context = unsafe { &mut *CURRENT_CONTEXT.get() };
    *current_context = new_context;

    unsafe {
        before_switch();
        restore_cpu_status(current_context)
    };
}

pub fn invoke_context_switch() {
    gic::set_pending(timer::TIMER_IRQ);
}
