use core::{any::Any, fmt::Debug};

use crate::{
    drivers::vfs::SeekOffset,
    process::{self, mem::MemMappedInterface, poll::PollID},
    warn,
};
use alloc::{boxed::Box, sync::Arc};
use hashbrown::HashMap;
use safa_abi::errors::ErrorStatus;

/// A resource ID
pub type Ri = u32;

pub trait Resource: Any {
    /// Performs a write operation on the resource.
    fn write(&self, off: SeekOffset, buf: &[u8]) -> Result<usize, ErrorStatus> {
        _ = off;
        _ = buf;
        Err(ErrorStatus::UnsupportedResource)
    }
    /// Performs a read operation on the resource.
    fn read(&self, off: SeekOffset, buf: &mut [u8]) -> Result<usize, ErrorStatus> {
        _ = off;
        _ = buf;
        Err(ErrorStatus::UnsupportedResource)
    }
    /// Performs a synchronization operation on the resource.
    fn sync(&self) -> Result<(), ErrorStatus> {
        Err(ErrorStatus::UnsupportedResource)
    }
    /// Performs a truncation operation on the resource.
    fn truncate(&self, len: usize) -> Result<(), ErrorStatus> {
        _ = len;
        Err(ErrorStatus::UnsupportedResource)
    }
    /// Performs a command operation on the resource.
    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
        _ = cmd;
        _ = arg;
        Err(ErrorStatus::UnsupportedResource)
    }

    /// Opens a memory mapping interface for the resource.
    fn open_mmap_interface(
        &self,
        offset: SeekOffset,
        page_count: usize,
    ) -> Result<Box<dyn MemMappedInterface>, ErrorStatus> {
        _ = offset;
        _ = page_count;
        Err(ErrorStatus::UnsupportedResource)
    }
    /// Attempts to create a new resource from the current one.
    fn try_clone_into_node(&self) -> Result<ResourceNodeRef, ErrorStatus> {
        Err(ErrorStatus::UnsupportedResource)
    }

    /// Whether the resource is sendable across address spaces.
    fn address_space_generic(&self) -> bool;
    /// Returns [`PollID`] for the resource's instance if it is pollable.
    fn poll_id(&self) -> Option<PollID> {
        None
    }
}

impl dyn Resource {
    #[inline]
    /// Attempts to downcast the resource to a specific type.
    pub fn as_ref<T: Resource>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }

    #[inline]
    /// Attempts to downcast the resource to a specific type, returning an error if it fails.
    pub fn as_ref_expected<T: Resource>(&self) -> Result<&T, ErrorStatus> {
        self.as_ref().ok_or(ErrorStatus::UnsupportedResource)
    }
}

/// Generic implementation of the `clone` method for resources
/// [`Resource::try_clone_into_node`]
pub fn generic_clone_impl<T: Resource + Clone>(
    resource: &T,
) -> Result<ResourceNodeRef, ErrorStatus> {
    Ok(ResourceNode::create(resource.clone()))
}

pub struct ResourceNodeInner<T: Resource + 'static + ?Sized> {
    data: T,
}

/// A Node that represents a resource in the process's resource tree.
pub type ResourceNode = ResourceNodeInner<dyn Resource>;
/// A shared reference to a [`ResourceNode`].
pub type ResourceNodeRef = Arc<ResourceNode>;

impl ResourceNode {
    pub fn create<T: Resource + 'static>(data: T) -> ResourceNodeRef {
        Arc::new(ResourceNodeInner { data: data })
    }

    pub fn data(&self) -> &dyn Resource {
        &self.data
    }

    pub fn cloneable_to_different_address_space(&self) -> bool {
        self.data.address_space_generic()
    }
}

pub struct ResourceManager {
    resources: HashMap<Ri, ResourceNodeRef>,
    next_resource_id: Ri,
}

impl Debug for ResourceManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceManager")
            .field("resources", &self.resources.keys())
            .finish()
    }
}

impl ResourceManager {
    pub fn new() -> Self {
        ResourceManager {
            resources: HashMap::new(),
            next_resource_id: 0,
        }
    }

    fn add_resource_node(&mut self, resource: ResourceNodeRef) -> Ri {
        let ri = self.next_resource_id;
        self.resources.insert(ri, resource);
        self.next_resource_id += 1;
        ri
    }

    pub fn add_global_resource<R: Resource + 'static>(&mut self, data: R) -> Ri {
        self.add_resource_node(ResourceNode::create(data))
    }

    #[inline]
    pub fn remove_resource(&mut self, ri: Ri) -> bool {
        // TODO: keep track of resource ids
        match self.resources.remove(&ri) {
            None => false,
            Some(_) => true,
        }
    }

    pub fn clone_resource(&mut self, ri: Ri) -> Option<Result<ResourceNodeRef, ErrorStatus>> {
        let resource = self.get_mut(ri)?;
        Some(resource.data().try_clone_into_node())
    }

    pub fn clone(&self) -> Self {
        let mut resources = HashMap::with_capacity(self.resources.capacity());
        for (res_id, res) in self.resources.iter() {
            if res.cloneable_to_different_address_space()
                && let Ok(res) = res.data().try_clone_into_node()
            {
                resources.insert(*res_id, res);
            }
        }

        Self {
            resources,
            next_resource_id: self.next_resource_id,
        }
    }
    /// Clones specific resources by their ids
    ///
    /// # Returns
    /// A resource manager containing only the `resource_ids` from self or an Err if a resource id isn't available
    pub fn clone_specific_resources(&mut self, resource_ids: &[Ri]) -> Result<ResourceManager, ()> {
        if resource_ids.is_empty() {
            return Ok(ResourceManager::new());
        }

        let mut new_resources = HashMap::new();
        let mut max_resource_id = 0;

        for resource_id in resource_ids {
            let resource_id = *resource_id;
            let result = self.clone_resource(resource_id).ok_or(())?;
            if let Ok(result) = result {
                new_resources.insert(resource_id, result);

                if max_resource_id < resource_id {
                    max_resource_id = resource_id;
                }
            }
        }

        Ok(Self {
            resources: new_resources,
            next_resource_id: max_resource_id + 1,
        })
    }

    /// gets a reference to the resource with index `ri`
    /// returns `None` if `ri` is invalid
    fn get<'s>(&'s self, ri: Ri) -> Option<&'s ResourceNodeRef> {
        self.resources.get(&ri)
    }

    fn get_mut(&mut self, ri: Ri) -> Option<&mut ResourceNodeRef> {
        self.resources.get_mut(&ri)
    }

    pub fn drop_all(&mut self) {
        for (ri, res) in self.resources.drain() {
            if Arc::strong_count(&res) != 1 {
                warn!(
                    ResourceManager,
                    "Resource at {ri} has {} references, process dropped...",
                    Arc::strong_count(&res)
                );
            }
        }
    }
}

impl Drop for ResourceManager {
    fn drop(&mut self) {
        self.drop_all();
    }
}

/// Gets a shared reference to the resource with the ID `ri`.
pub fn get(ri: Ri) -> Option<ResourceNodeRef> {
    let this = process::current();
    this.resources().get(ri).cloned()
}

/// Same as [`get`] but returns an error if the resource is not found.
pub fn get_expected(ri: Ri) -> Result<ResourceNodeRef, ErrorStatus> {
    get(ri).ok_or(ErrorStatus::UnknownResource)
}

/// Like [`get`] but instead of returning a shared reference, it uses a closure to execute on the resource, keeps a lock held as long as the closure is executing.
///
/// If you are going to do something potentially blocking or slow use [`get`] instead
pub fn get_ref<DO, R>(ri: Ri, then: DO) -> Option<R>
where
    DO: FnOnce(&ResourceNode) -> R,
{
    let this = process::current();
    this.resources_mut().get(ri).map(|r| then(r))
}

/// Adds a resource that lives as long as the current process, to the current process
pub fn add_global_resource<R: Resource + 'static>(resource_data: R) -> Ri {
    let this = process::current();
    this.resources_mut().add_global_resource(resource_data)
}

/// Duplicates a resource return the new duplicate resource's ID or None if that resource doesn't exist
pub fn duplicate_resource(ri: Ri) -> Option<Result<Ri, ErrorStatus>> {
    let current_process = process::current();
    let mut manager = current_process.resources_mut();

    let resource = manager.clone_resource(ri)?;
    match resource {
        Ok(resource) => Some(Ok(manager.add_resource_node(resource))),
        Err(err) => Some(Err(err)),
    }
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: Ri) -> bool {
    let current_process = process::current();
    current_process.resources_mut().remove_resource(ri)
}
