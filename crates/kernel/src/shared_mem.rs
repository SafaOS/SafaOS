use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use hashbrown::HashMap;
use lazy_static::lazy_static;

use crate::{
    error,
    memory::frame_allocator::{self, Frame},
    process::vas::MemMappedInterface,
    utils::locks::Mutex,
};

/// A generated Shared Memory Key, that is different for each Shared Memory Descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ShmKey(pub usize);

/// A Shared Memory Descriptor, Shared Memory is special memory that can be `SysMemMap`ped from different processes,
/// so that they can share memory in a form of IPC.
pub struct SharedMemoryDesc {
    frames: Box<[Frame]>,
    tracked_references: AtomicUsize,
}

impl SharedMemoryDesc {
    /// Allocates `pages` pages of memory and returns a Shared Memory Descriptor that owns that Memory
    /// returns an Error if memory allocation fails
    pub fn allocate(pages: usize) -> Result<Self, ()> {
        let mut frames = Vec::with_capacity(pages);

        for _ in 0..pages {
            let frame = frame_allocator::allocate_frame();
            if let Some(frame) = frame {
                frames.push(frame);
            } else {
                error!(SharedMemoryDesc, "OOM allocating {pages} Page(s)");
                for frame in frames {
                    frame_allocator::deallocate_frame(frame);
                }

                return Err(());
            }
        }

        Ok(Self {
            frames: frames.into_boxed_slice(),
            tracked_references: AtomicUsize::new(0),
        })
    }

    /// Returns the frames this Shared Memory Descriptor uses internally, on Drop these frames are deallocated
    fn frames(&self) -> &[Frame] {
        &self.frames
    }
}

impl Drop for SharedMemoryDesc {
    fn drop(&mut self) {
        for frame in &self.frames {
            frame_allocator::deallocate_frame(*frame);
        }
    }
}

/// A memory mapped interface over a [`SharedMemoryDesc`]
struct SharedMemoryMap {
    descriptor: Arc<SharedMemoryDesc>,
}

impl MemMappedInterface for SharedMemoryMap {
    fn frames(&self) -> &[Frame] {
        self.descriptor.frames()
    }
}

/// Manages Shared Memory, see [`SharedMemoryDesc`]
pub struct ShmManager {
    descriptors: HashMap<ShmKey, Arc<SharedMemoryDesc>>,
    next_shm_key: ShmKey,
}

impl ShmManager {
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
            next_shm_key: ShmKey(0),
        }
    }

    pub fn allocate(&mut self, pages_count: usize) -> Result<(Arc<SharedMemoryDesc>, ShmKey), ()> {
        let key = self.next_shm_key;
        let desc = SharedMemoryDesc::allocate(pages_count).map(Arc::new)?;
        self.descriptors.insert(key, desc.clone());

        self.next_shm_key = ShmKey(key.0 + 1);
        Ok((desc, key))
    }

    pub fn remove(&mut self, key: ShmKey) -> bool {
        self.descriptors.remove(&key).is_some()
    }

    fn get_descriptor(&self, key: ShmKey) -> Option<Arc<SharedMemoryDesc>> {
        self.descriptors.get(&key).cloned()
    }
}

lazy_static! {
    static ref SHM_TABLE: Mutex<ShmManager> = Mutex::new(ShmManager::new());
}

/// A [`ShmKey`] that drops the descriptor when all of it's instances that uses the same [`ShmKey`] goes out of scope
pub struct TrackedShmKey {
    desc: Arc<SharedMemoryDesc>,
    key: ShmKey,
}

impl TrackedShmKey {
    /// Tracks a given descriptor, drops it when all the references are gone
    pub fn track(desc: Arc<SharedMemoryDesc>, key: ShmKey) -> Self {
        desc.tracked_references.fetch_add(1, Ordering::SeqCst);
        Self {
            desc: desc.clone(),
            key,
        }
    }

    /// Returns the key self contains, when self is dropped this key becomes unusable, but it won't cause UB so it is safe
    pub const fn key(&self) -> &ShmKey {
        &self.key
    }
    /// Given a [`TrackedShmKey`] attempt to create a memory mapped interface over the [`SharedMemoryDescriptor`] the key points to,
    pub fn mmap_interface(&self) -> Box<dyn MemMappedInterface> {
        let descriptor = self.desc.clone();
        Box::new(SharedMemoryMap { descriptor })
    }
}

impl Drop for TrackedShmKey {
    fn drop(&mut self) {
        if self.desc.tracked_references.fetch_sub(1, Ordering::SeqCst) <= 1 {
            assert!(
                SHM_TABLE.lock().remove(self.key),
                "Attempt to double free a SHM Descriptor"
            )
        }
    }
}

impl Clone for TrackedShmKey {
    fn clone(&self) -> Self {
        Self::track(self.desc.clone(), self.key)
    }
}

/// Create a [`SharedMemoryDesc`] and returns it's tracked key (see [`TrackedShmKey`]),
/// You can then use the key to map that descriptor to a [`SharedMemoryMap`].
///
/// Returns an Err(()) if out of memory.
///
/// To make the descriptor free you just have to drop the key, and then wait for all Memory Mapped Interfaces over it to drop too.
pub fn create_shm(pages_count: usize) -> Result<TrackedShmKey, ()> {
    SHM_TABLE
        .lock()
        .allocate(pages_count)
        .map(|(desc, key)| TrackedShmKey::track(desc, key))
}

/// Creates a [`TrackedShmKey`] that points to the Shared Memory Descriptor `key` points to,
/// or returns None if the key is invalid
pub fn track_shm(key: ShmKey) -> Option<TrackedShmKey> {
    SHM_TABLE
        .lock()
        .get_descriptor(key)
        .map(|desc| TrackedShmKey::track(desc, key))
}
