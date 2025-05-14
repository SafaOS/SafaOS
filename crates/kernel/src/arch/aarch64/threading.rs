use core::arch::{asm, global_asm};

use safa_utils::abi::raw::processes::AbiStructures;

use crate::{
    arch::aarch64::{gic, timer},
    memory::paging::{MapToError, PhysPageTable},
    threading, PhysAddr, VirtAddr,
};

use super::exceptions::InterruptFrame;

/// The CPU Status for each thread (registers)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CPUStatus {
    ttbr0: PhysAddr,
    sp: VirtAddr,
    frame: InterruptFrame,
}

impl CPUStatus {
    fn new(frame: &mut InterruptFrame, page_table_addr: PhysAddr, sp: VirtAddr) -> Self {
        Self {
            frame: *frame,
            ttbr0: page_table_addr,
            sp,
        }
    }
    unsafe fn from_current(frame: &mut InterruptFrame) -> Self {
        let ttbr0: PhysAddr;
        let sp_el0: PhysAddr;
        unsafe {
            asm!("mrs {}, sp_el0; mrs {}, ttbr0_el1", out(reg) sp_el0, out(reg) ttbr0);
        }

        Self::new(frame, ttbr0, sp_el0)
    }
}

global_asm!(
    "
.text
.global restore_cpu_partial
# restores only the translation table and the stack
restore_cpu_partial:
  ldp x1, x2, [x0]
  # x0 has to be a higher half address or everything breaks....
  # loads the translation table and the stack pointer
  msr ttbr0_el1, x1
  # reload address space
  tlbi VMALLE1
  dsb ISH
  isb
  msr sp_el0, x2
  ret
.global restore_cpu_status
restore_cpu_status:
    bl restore_cpu_partial
    mov x1, #0x10
    add x0, x0, x1
    b restore_frame
"
);

extern "C" {
    ///  Takes a reference to [`CPUStatus`] and sets current cpu status (registers) to it
    pub fn restore_cpu_status(status: &CPUStatus) -> !;
    /// Restores everything from the cpu status expect for the InterruptFrame
    /// used by the timer interrupt because it is responsible to restore it's own frame
    fn restore_cpu_partial(status: &CPUStatus);
}

impl CPUStatus {
    #[allow(unused_variables)]
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
        unimplemented!()
    }

    pub fn at(&self) -> VirtAddr {
        *self.frame.elr as VirtAddr
    }

    pub fn stack_at(&self) -> VirtAddr {
        todo!()
    }
}

pub(super) unsafe fn context_switch(frame: &mut InterruptFrame) {
    let context = unsafe { CPUStatus::from_current(frame) };
    let new_context = threading::swtch(context);
    unsafe { restore_cpu_partial(&new_context) };
    *frame = new_context.frame;
}

pub fn invoke_context_switch() {
    gic::set_pending(timer::TIMER_IRQ);
}
