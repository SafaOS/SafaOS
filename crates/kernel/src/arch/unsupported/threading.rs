#![allow(unreachable_code)]
use safa_utils::abi::raw::processes::AbiStructures;

use crate::{
    memory::paging::{MapToError, PhysPageTable},
    VirtAddr,
};

/// The CPU Status for each thread (registers)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct CPUStatus(!);
extern "C" {
    ///  Takes a reference to [`CPUStatus`] and sets current cpu status (registers) to it
    pub fn restore_cpu_status(status: &CPUStatus) -> !;
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
        self.0
    }

    pub fn stack_at(&self) -> VirtAddr {
        self.0
    }
}

#[inline(always)]
pub fn invoke_context_switch() {
    todo!()
}
