use macros::syscall_handler;
use safa_abi::arch::ArchOp;

use crate::{
    VirtAddr,
    syscalls::{ErrorStatus, ffi::SyscallFFI},
};

impl SyscallFFI for ArchOp {
    type Args = <u32 as SyscallFFI>::Args;
    fn make(args: Self::Args) -> Result<Self, safa_abi::errors::ErrorStatus> {
        Self::try_from(u32::make(args)?).ok_or(ErrorStatus::InvalidArgument)
    }
}

#[syscall_handler]
fn sysarch_ctrl(op: ArchOp, arg: u64) -> Result<(), ErrorStatus> {
    match op {
        ArchOp::None => Ok(()),
        ArchOp::X86SetFS => {
            cfg_if::cfg_if! {
                if #[cfg(target_arch = "x86_64")] {
                    unsafe { crate::arch::x86_64::registers::wrfsbase(VirtAddr::from(arg as usize)) };
                    Ok(())
                } else {
                    Err(ErrorStatus::OperationNotSupported)
                }
            }
        }
    }
}
