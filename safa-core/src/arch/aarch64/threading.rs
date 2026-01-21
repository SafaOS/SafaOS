use core::arch::{asm, global_asm};

use crate::{
    PhysAddr, VirtAddr,
    memory::paging::{MapToError, PhysPageTable},
    scheduler::{self},
    thread::Tid,
};

use super::{
    exceptions::InterruptFrame,
    registers::{Reg, Spsr},
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
    # Ensure writes are visible before reloading address space
    dsb ish
    isb

    # reload address space
    tlbi VMALLE1
    dsb ish
    ISB

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
        tls_addr: VirtAddr,
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
                tpidr_el0: Reg(tls_addr.into_raw() as u64),
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
        tls_addr: VirtAddr,
        user_stack_end: VirtAddr,
        kernel_stack_end: VirtAddr,
        page_table: &mut PhysPageTable,
        entry_point: VirtAddr,
        thread_id: Tid,
        arguments_ptr: *const (),
        userspace: bool,
    ) -> Result<Self, MapToError> {
        let el0_stack_end = user_stack_end;
        let el1_stack_end = kernel_stack_end;

        let mut general_registers = [Reg::default(); 29];
        general_registers[0] = Reg(thread_id as u64);
        general_registers[1] = Reg(arguments_ptr as u64);

        Ok(Self {
            ttbr0: page_table.phys_addr(),
            sp_el0: el0_stack_end,
            frame: InterruptFrame {
                general_registers,
                tpidr_el0: Reg(tls_addr.into_raw() as u64),
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

unsafe extern "C" {
    fn context_switch_and_return();
}

#[unsafe(no_mangle)]
extern "C" fn context_switch_now(frame: &mut InterruptFrame) -> ! {
    let context = unsafe { CPUStatus::from_current(frame) };
    let swtch_results = scheduler::swtch(context);

    if let Some((new_context_ptr, address_space_changed)) = swtch_results {
        unsafe {
            if !address_space_changed {
                restore_cpu_status_partial(new_context_ptr.as_ref());
            } else {
                restore_cpu_status(new_context_ptr.as_ref());
            }
        }
    } else {
        core::hint::cold_path();
        unsafe { restore_cpu_status_partial(&context) };
    }
}

#[inline]
pub(super) unsafe fn context_switch(frame: &mut InterruptFrame, before_switch: impl FnOnce()) {
    let context = unsafe { CPUStatus::from_current(frame) };
    let swtch_results = scheduler::swtch(context);

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

#[inline]
pub fn invoke_context_switch() {
    unsafe { context_switch_and_return() }
}
