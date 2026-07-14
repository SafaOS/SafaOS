use core::{num::NonZero, sync::atomic::AtomicU32};

use macros::syscall_handler;
use safa_abi::{
    errors::ErrorStatus,
    process::{RawContextPriority, RawTSpawnConfig},
};

use crate::{
    VirtAddr, process,
    scheduler::wait_queue::WaitError,
    thread::{self, ContextPriority},
};
use crate::{syscalls::SyscallFFI, thread::Tid};

#[syscall_handler]
fn syst_fut_wake(addr: &AtomicU32, n: usize) -> usize {
    process::current::wake_futex(addr, n)
}

#[syscall_handler]
fn syst_fut_wait(addr: &AtomicU32, val: u32, timeout_ms: u64) -> Result<(), WaitError> {
    unsafe { thread::current::wait_for_futex(addr, val, timeout_ms) }
}

#[syscall_handler]
fn sys_tspawn(entry_point: VirtAddr, raw_config: &RawTSpawnConfig) -> Result<Tid, ErrorStatus> {
    let config: TSpawnConfig = raw_config.try_into()?;

    let thread_tid = process::current::thread_spawn(
        entry_point,
        config.argument_ptr,
        config.priority,
        config.cpu.map(|v| v as usize /* too lazy to change */),
        config.custom_stack_size,
    )
    .map_err(|_| ErrorStatus::MMapError)?;

    Ok(thread_tid)
}

struct TSpawnConfig {
    argument_ptr: VirtAddr,
    priority: Option<ContextPriority>,
    cpu: Option<u8>,
    custom_stack_size: Option<NonZero<usize>>,
}

impl TryFrom<&RawTSpawnConfig> for TSpawnConfig {
    type Error = ErrorStatus;
    fn try_from(value: &RawTSpawnConfig) -> Result<Self, Self::Error> {
        let argument_ptr = VirtAddr::from_ptr(value.argument_ptr);
        let priority = match value.priority {
            RawContextPriority::Default => None,
            RawContextPriority::Medium => Some(ContextPriority::Medium),
            RawContextPriority::Low => Some(ContextPriority::Low),
            RawContextPriority::High => Some(ContextPriority::High),
        };

        let cpu = value.cpu.into();
        let custom_stack_size = if value.revision >= 1 {
            value.custom_stack_size.into()
        } else {
            None
        };

        Ok(TSpawnConfig {
            argument_ptr,
            priority,
            cpu,
            custom_stack_size,
        })
    }
}
