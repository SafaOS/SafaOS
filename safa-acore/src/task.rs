use core::{alloc::Layout, cell::UnsafeCell, pin::Pin, ptr::NonNull};

use alloc::{
    alloc::{AllocError, Global},
    boxed::Box,
    sync::Arc,
};
use futures_util::task::{ArcWake, WakerRef};
use thiserror::Error;

use crate::{
    mem::local_arena::{self, LocalArena},
    scheduler::{self, current_task_ref},
};

pub type KeStackArena = LocalArena<8072>;
const _: () = assert!(KeStackArena::SIZE_OF_ARENA == 8096);

/// Represents a task
pub struct Task {
    pub(crate) future: UnsafeCell<Pin<Box<dyn Future<Output = ()> + Send>>>,
    ke_stack: KeStackArena,
}

impl Task {
    #[inline(always)]
    /// Returns a reference to a waker that will wake this task no doubt.
    pub fn waker_ref<'s>(self: &'s Arc<Self>) -> WakerRef<'s> {
        futures_util::task::waker_ref(&self)
    }
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl ArcWake for Task {
    fn wake(self: Arc<Self>) {
        scheduler::wake_task(self);
    }
    fn wake_by_ref(arc_self: &Arc<Self>) {
        scheduler::wake_task(arc_self.clone());
    }
}

#[derive(Debug, Clone, Copy, Error)]
pub enum TaskSpawnError {
    #[error("Resource Allocation failed for summoning a task: {0}")]
    AllocError(#[from] AllocError),
}

/// Attempts to spawn a kernel task, the task my immediately run in the background.
pub fn try_spawn<F: Future<Output = ()> + 'static + Send>(fut: F) -> Result<(), TaskSpawnError> {
    scheduler::new_task(Arc::try_new(Task {
        future: UnsafeCell::new(Box::into_pin(Box::try_new(fut)?)),
        ke_stack: KeStackArena::try_new_in(Global)?,
    })?);

    Ok(())
}

#[unsafe(no_mangle)]
extern "C" fn SafaRT_curr_stack_ptr() -> NonNull<()> {
    // Safety: Reference isn't sent to another thread.
    unsafe {
        current_task_ref()
            .expect("Attempt to get a pointer to kernel stack without any task running")
            .ke_stack
            .arena_ptr()
            .cast()
    }
}

#[unsafe(no_mangle)]
extern "C" fn SafaRT_stack_alloc(
    ptr_to_stack: NonNull<()>,
    size: usize,
    align: usize,
) -> Result<NonNull<u8>, ()> {
    let layout = Layout::from_size_align(size, align).expect("SafaRT_stack_alloc: invalid layout");
    let arena_ptr = unsafe {
        current_task_ref()
            .expect("Attempt to pop from kernel stack without any task running")
            .ke_stack
            .arena_ptr()
    };

    assert!(
        core::ptr::eq(arena_ptr.as_ptr(), ptr_to_stack.as_ptr().cast()),
        "Attempt to allocate an object from the kernel stack of a thread after it was moved to another thread"
    );

    // Safety: Reference isn't sent to another thread.
    unsafe {
        current_task_ref()
            .expect("Attempt to allocate on kernel stack without any task running")
            .ke_stack
            .allocate(layout)
            .map(|p| p.cast())
            .map_err(|AllocError| ())
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn SafaRT_stack_free(
    ptr_to_stack: NonNull<()>,
    data_ptr: NonNull<u8>,
    size: usize,
    align: usize,
) {
    let layout = Layout::from_size_align(size, align).expect("SafaRT_stack_free: invalid layout");
    let arena_ptr = unsafe {
        current_task_ref()
            .expect("Attempt to pop from kernel stack without any task running")
            .ke_stack
            .arena_ptr()
    };

    assert!(
        core::ptr::eq(arena_ptr.as_ptr(), ptr_to_stack.as_ptr().cast()),
        "Attempt to pop an object from the kernel stack of a thread after it was moved to another thread"
    );

    unsafe { local_arena::deallocate(arena_ptr, data_ptr, layout) };
}
