use core::fmt::Debug;

use crate::{
    drivers::vfs::CollectionIterDescriptor,
    utils::locks::{Mutex, MutexGuard},
};
use alloc::vec::Vec;

use crate::drivers::vfs::FSObjectDescriptor;

use super::expose::thread_yield;

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
    pub fn new() -> Self {
        ResourceManager {
            resources: Vec::with_capacity(2),
            next_ri: 0,
        }
    }

    fn add_resource(&mut self, resource: Resource) -> usize {
        let resources = &mut self.resources[self.next_ri..];

        for (ri, res) in resources.iter_mut().enumerate() {
            let res = res.get_mut();
            if matches!(*res, Resource::Null) {
                let ri = self.next_ri + ri;

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

    #[inline(always)]
    fn remove_resource(&mut self, ri: usize) -> Option<()> {
        if ri >= self.resources.len() {
            return None;
        }

        loop {
            if let Some(resource) = self.resources.get_mut(ri).map(|r| r.get_mut()) {
                *resource = Resource::Null;
                break;
            }

            thread_yield();
        }

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
    let current = super::current();
    let state = current.state().unwrap();

    state.resource_manager().unwrap().get(ri).map(then)
}

/// adds a resource to the current process
pub fn add_resource(resource: Resource) -> usize {
    let current_task = super::current();
    let mut current = current_task.state_mut().unwrap();

    current
        .resource_manager_mut()
        .unwrap()
        .add_resource(resource)
}

pub fn duplicate_resource(ri: usize) -> usize {
    let mut state = super::this_state_mut();
    let manager = state.resource_manager_mut().unwrap();

    let resource = manager.get_mut(ri).unwrap();
    let clone = resource.clone();
    manager.add_resource(clone)
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: usize) -> Option<()> {
    let current_task = super::current();
    let mut current = current_task.state_mut().unwrap();

    current.resource_manager_mut().unwrap().remove_resource(ri)
}
