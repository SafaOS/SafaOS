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

    pub fn next_ri(&self) -> usize {
        self.next_ri
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

    pub fn clone_resource(&mut self, ri: usize) -> Option<ResourceItem> {
        if ri >= self.resources.len() {
            return None;
        }

        Some(Mutex::new(self.resources[ri].get_mut().clone()))
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
    let state = this.state();

    state
        .resource_manager()
        .expect("tried to get a resource in a dead process (process)")
        .get(ri)
        .map(then)
}

/// adds a resource to the current process
pub fn add_resource(resource: Resource) -> usize {
    let this = process::current();
    let mut state = this.state_mut();

    state
        .resource_manager_mut()
        .expect("tried to add a resource in a dead process")
        .add_resource(resource)
}

pub fn duplicate_resource(ri: usize) -> usize {
    let current_process = process::current();
    let mut state = current_process.state_mut();
    let manager = state.resource_manager_mut().unwrap();

    let resource = manager.get_mut(ri).unwrap();
    let clone = resource.clone();
    manager.add_resource(clone)
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: usize) -> Option<()> {
    let current_process = process::current();
    let mut current = current_process.state_mut();

    current
        .resource_manager_mut()
        .expect("tried to remove a resource in a dead process")
        .remove_resource(ri)
}
