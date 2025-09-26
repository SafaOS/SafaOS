use core::{num::NonZero, ptr::NonNull};

use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use safa_abi::errors::IntoErr;
use smallvec::SmallVec;

use crate::{
    arch::with_interrupts, process::resources, scheduler::wait_queue::WaitQueueWithTimeout, thread,
    utils::locks::Mutex,
};

pub use safa_abi::poll::PollEvents;

/// A unique identifier for a resource in a poll operation,
/// This identifier could be a pointer to the resource for example, the only thing that matters is that it is unique per shared resource instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct PollID(usize);

impl PollID {
    pub fn from_ptr<T>(ptr: *const T) -> Self {
        Self(ptr as usize)
    }

    pub const fn from_usize(value: usize) -> Self {
        Self(value)
    }
}

pub struct PollEntry {
    resources: SmallVec<[(PollID, PollEvents); 3]>,
}

pub struct ResourcePoll {
    status: HashMap<PollID, PollEvents, FxBuildHasher>,
    // For now the size doesn't matter
    queue: WaitQueueWithTimeout<8, NonNull<PollEntry>>,
}

unsafe impl Send for ResourcePoll {}

impl ResourcePoll {
    pub const fn new() -> Self {
        Self {
            status: HashMap::with_hasher(FxBuildHasher),
            queue: WaitQueueWithTimeout::new(),
        }
    }

    fn current_events(&self, id: PollID) -> Option<PollEvents> {
        self.status.get(&id).copied()
    }

    fn broadcast_events(&mut self, id: PollID, events_add: PollEvents, events_remove: PollEvents) {
        let old_events = self.status.get(&id).copied().unwrap_or(PollEvents::NONE);
        let events_all = old_events.difference(events_remove).union(events_add);

        self.status.insert(id, events_all);
        if events_all.contains(PollEvents::DISCONNECTED) {
            self.wake_id_with_reasons(id, events_all);
        } else if !events_all.is_empty() {
            self.wake_for_id_reasons(id, events_all);
        }
    }

    fn stop_tracking_id(&mut self, id: PollID) {
        self.status.remove(&id);
        self.wake_id_with_reasons(id, PollEvents::DISCONNECTED);
    }

    fn wake_on_condition(&mut self, mut condition: impl FnMut(&mut PollEntry) -> bool) {
        self.queue.wake_on_condition(|entry| {
            let entry = unsafe { entry.as_mut() };
            condition(entry)
        });
    }

    pub fn wake_for_id_reasons(&mut self, id: PollID, reasons: PollEvents) {
        self.wake_on_condition(|ent| {
            let mut results = false;

            for (e_id, r) in &mut ent.resources {
                if *e_id == id && r.intersects(reasons) {
                    *r = r.intersection(reasons);
                    results = true;
                }
            }
            results
        });
    }

    pub fn wake_id_with_reasons(&mut self, id: PollID, reasons: PollEvents) {
        self.wake_on_condition(|ent| {
            let mut results = false;

            for (e_id, r) in &mut ent.resources {
                if *e_id == id {
                    *r = reasons;
                    results = true;
                }
            }
            results
        });
    }
}

static POLL_QUEUE: Mutex<ResourcePoll> = Mutex::new(ResourcePoll::new());

/// Broadcasts events to all tasks waiting on the [`PollID`] `id`.
pub fn broadcast_events(poll_id: PollID, events_add: PollEvents, events_remove: PollEvents) {
    POLL_QUEUE
        .lock()
        .broadcast_events(poll_id, events_add, events_remove);
}

/// Removes the [`PollID`] `id` from the poll queue.
pub fn stop_tracking_id(id: PollID) {
    POLL_QUEUE.lock().stop_tracking_id(id);
}

/// An error that can occur when attempting to poll resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollError {
    /// Resource doesn't exist.
    UnknownResource,
    /// Resource doesn't support polling.
    UnsupportedResource,
}

impl IntoErr for PollError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::UnknownResource => safa_abi::errors::ErrorStatus::UnknownResource,
            Self::UnsupportedResource => safa_abi::errors::ErrorStatus::UnsupportedResource,
        }
    }
}

/// Performs a poll operation on the given resources.
///
/// returns immediately if no resources are provided or if timeout is 0 or reached.
pub fn poll_resources(
    entries: &mut [safa_abi::poll::PollEntry],
    timeout_after_ms: u64,
) -> Result<(), PollError> {
    if entries.is_empty() {
        return Ok(());
    }

    let Some(timeout_after) = NonZero::new(timeout_after_ms) else {
        return Ok(());
    };
    let timeout_after = (timeout_after.get() != u64::MAX).then_some(timeout_after);

    let mut poll_queue = POLL_QUEUE.lock();
    let mut any_skipped = false;
    let mut actual_count = 0;

    for ent in &mut *entries {
        let ri = ent.resource();
        let poll_for = ent.events();
        let poll_result = ent.returned_events_mut();

        if poll_for.is_empty() {
            continue;
        }

        let poll_id = resources::get_ref(ri, |res| {
            res.data().poll_id().ok_or(PollError::UnsupportedResource)
        })
        .ok_or(PollError::UnknownResource)
        .flatten()?;

        if let Some(status) = poll_queue.current_events(poll_id) {
            if status.contains(poll_for) || status.contains(PollEvents::DISCONNECTED) {
                *poll_result = status;
                any_skipped = true;
            }
        } else if !any_skipped {
            *poll_result = PollEvents::NONE;
        }

        actual_count += 1;
    }

    if any_skipped || actual_count == 0 {
        return Ok(());
    }

    let mut queue_entry = ::alloc::boxed::Box::new(PollEntry {
        resources: SmallVec::with_capacity(actual_count),
    });

    for ent in &mut *entries {
        let ri = ent.resource();
        let poll_for = ent.events();

        if poll_for.is_empty() {
            continue;
        }

        let poll_id = resources::get_ref(ri, |res| {
            res.data().poll_id().ok_or(PollError::UnsupportedResource)
        })
        .ok_or(PollError::UnknownResource)
        .flatten()?;

        queue_entry.resources.push((poll_id, poll_for));
    }

    let entry_ptr = NonNull::from_ref(&*queue_entry);

    with_interrupts(move || {
        thread::current().sleep_in_queue_with_timeout(
            &mut poll_queue.queue,
            entry_ptr,
            timeout_after,
        );
        drop(poll_queue);
        thread::current::yield_now();
    });

    // Once we are awaken entry should be filled with the results
    let mut results = queue_entry.resources.iter().map(|(_, result)| *result);
    for entry in entries {
        let poll_for = entry.events();
        let poll_res = entry.returned_events_mut();

        if poll_for.is_empty() /* skipped */ || !poll_res.is_empty()
        /* results already supplied */
        {
            continue;
        }

        /* supply results */
        *poll_res = results.next().unwrap();
    }
    Ok(())
}
