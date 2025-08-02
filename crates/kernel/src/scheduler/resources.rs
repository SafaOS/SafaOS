use core::fmt::Debug;

use crate::{
    drivers::vfs::CollectionIterDescriptor,
    process,
    utils::locks::{Mutex, MutexGuard},
};
use alloc::vec::Vec;

use crate::drivers::vfs::FSObjectDescriptor;

#[derive(Clone)]
pub enum Resource {
    Null,
    File(FSObjectDescriptor),
    DirIter(CollectionIterDescriptor),
}

type ResourceItem = Mutex<Resource>;
pub struct ResourceManager {
    resources: Vec<ResourceItem>,
    next_ri: usize,
}

impl Debug for ResourceManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceManager")
            .field(
                "resources",
                &self
                    .resources
                    .iter()
                    .enumerate()
                    .map(|(i, _)| i)
                    .collect::<Vec<usize>>(),
            )
            .field("next_ri", &self.next_ri)
            .finish()
    }
}

impl ResourceManager {
    pub const fn new() -> Self {
        ResourceManager {
            resources: Vec::new(),
            next_ri: 0,
        }
    }

    fn add_resource(&mut self, resource: Resource) -> usize {
        for (ri, res) in self.resources.iter_mut().enumerate().skip(self.next_ri) {
            let res = res.get_mut();
            if matches!(*res, Resource::Null) {
                self.next_ri = ri;
                *res = resource;

                return ri;
            }
        }

        self.resources.push(Mutex::new(resource));

        let ri = self.resources.len() - 1;
        self.next_ri = ri;

        ri
    }

    #[inline]
    fn remove_resource(&mut self, ri: usize) -> Option<()> {
        let resource = self.resources.get_mut(ri).map(|r| r.get_mut())?;
        *resource = Resource::Null;

        if ri < self.next_ri {
            self.next_ri = ri;
        }

        Some(())
    }

    pub fn overwrite_resources(&mut self, resources: Vec<ResourceItem>) {
        self.resources = resources;
    }

    pub fn clone_resources(&mut self) -> Vec<ResourceItem> {
        self.resources
            .iter_mut()
            .map(|r| Mutex::new(r.get_mut().clone()))
            .collect()
    }

    pub fn clone_resource(&self, ri: usize) -> Option<ResourceItem> {
        if ri >= self.resources.len() {
            return None;
        }

        Some(Mutex::new(self.resources[ri].lock().clone()))
    }
    /// Clones specific resources by their ids
    ///
    /// # Returns
    /// A vector of cloned resources corresponding to the provided ids if successful, otherwise an Err(()) if any resource doesn't exist
    pub fn clone_specific_resources(
        &self,
        resource_ids: &[usize],
    ) -> Result<Vec<ResourceItem>, ()> {
        if resource_ids.is_empty() {
            return Ok(Vec::new());
        }

        let biggest = resource_ids.iter().max().copied().unwrap_or(0);
        // ensures the results has the same ids as the resources
        let mut results = Vec::with_capacity(biggest + 1);
        results.resize_with(biggest + 1, || Mutex::new(Resource::Null));

        for resource_id in resource_ids {
            let resource_id = *resource_id;

            let result = self.clone_resource(resource_id).ok_or(())?;
            results[resource_id] = result;
        }

        Ok(results)
    }

    /// gets a reference to the resource with index `ri`
    /// returns `None` if `ri` is invalid
    fn get<'s>(&'s self, ri: usize) -> Option<MutexGuard<'s, Resource>> {
        if ri >= self.resources.len() {
            return None;
        }

        Some(self.resources[ri].lock())
    }

    fn get_mut(&mut self, ri: usize) -> Option<&mut Resource> {
        if ri >= self.resources.len() {
            return None;
        }
        Some(self.resources[ri].get_mut())
    }
}
// TODO: fgure out a better way to do this, where it's easier to tell that we are holding a lock on
// the current process state.

/// gets a resource with ri `ri` then executes then on it
pub fn get_resource<DO, R>(ri: usize, then: DO) -> Option<R>
where
    DO: FnOnce(MutexGuard<Resource>) -> R,
{
    let this = process::current();
    this.resources().get(ri).map(then)
}

/// adds a resource to the current process
pub fn add_resource(resource: Resource) -> usize {
    let this = process::current();
    this.resources_mut().add_resource(resource)
}

pub fn duplicate_resource(ri: usize) -> usize {
    let current_process = process::current();
    let mut manager = current_process.resources_mut();

    let resource = manager.get_mut(ri).unwrap();
    let clone = resource.clone();
    manager.add_resource(clone)
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: usize) -> Option<()> {
    let current_process = process::current();
    current_process.resources_mut().remove_resource(ri)
}
