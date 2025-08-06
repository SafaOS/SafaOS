use core::{num::NonZero, sync::atomic::AtomicU32};

use macros::syscall_handler;
use safa_abi::{
    errors::ErrorStatus,
    process::{RawContextPriority, RawTSpawnConfig},
};

use crate::{
    VirtAddr, process,
    thread::{self, ContextPriority},
};
use crate::{syscalls::SyscallFFI, thread::Tid};

#[syscall_handler]
fn syst_fut_wake(addr: &AtomicU32, n: usize, wake_results: Option<&mut usize>) {
    let num_threads = process::current::wake_futex(addr, n);
    if let Some(wake_results) = wake_results {
        *wake_results = num_threads;
    }
}

#[syscall_handler]
fn syst_fut_wait(addr: &AtomicU32, val: u32, timeout_ms: u64, wait_results: Option<&mut bool>) {
    let results = unsafe { thread::current::wait_for_futex(addr, val, timeout_ms) };
    if let Some(wait_results) = wait_results {
        *wait_results = results;
    }
}

#[syscall_handler]
fn sys_tspawn(
    entry_point: VirtAddr,
    raw_config: &RawTSpawnConfig,
    target_tid: Option<&mut Tid>,
) -> Result<(), ErrorStatus> {
    let config: TSpawnConfig = raw_config.try_into()?;

    let thread_tid = process::current::thread_spawn(
        entry_point,
        config.argument_ptr,
        config.priority,
        config.cpu.map(|v| v as usize /* too lazy to change */),
        config.custom_stack_size,
    )
    .map_err(|_| ErrorStatus::MMapError)?;
    if let Some(target_tid) = target_tid {
        *target_tid = thread_tid;
    }
    Ok(())
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
