use alloc::alloc::AllocError;
use thiserror::Error;

use crate::{ffi::FFIResult, future::SendDynFut};

#[derive(Debug, Clone, Copy, Error)]
#[repr(u8)]
pub enum TaskSpawnError {
    #[error("Resource Allocation failed for spawning a task")]
    AllocError,
}

impl From<AllocError> for TaskSpawnError {
    fn from(value: AllocError) -> Self {
        let AllocError = value;
        Self::AllocError
    }
}

unsafe extern "C" {
    fn SafaRT_task_spawn(fut: SendDynFut<'static, ()>) -> FFIResult<(), TaskSpawnError>;
}

/// Attempts to spawn a kernel task, the task may immediately run in the background.
pub fn try_spawn<F: Future<Output = ()> + 'static + Send>(fut: F) -> Result<(), TaskSpawnError> {
    unsafe { SafaRT_task_spawn(SendDynFut::try_new(fut)?)? };
    Ok(())
}
