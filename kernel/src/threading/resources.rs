use core::fmt::Debug;

use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};

use crate::drivers::vfs::{DirIter, FileDescriptor};

use super::expose::thread_yeild;

#[derive(Clone)]
pub enum Resource {
    Null,
    File(FileDescriptor),
    DirIter(DirIter),
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

            thread_yeild();
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

    pub fn clone_resources(&self) -> Vec<ResourceItem> {
        let mut clone_resources = Vec::with_capacity(self.resources.len());

        for resource in &self.resources {
            let clone_resource = resource.lock().clone();
            clone_resources.push(Mutex::new(clone_resource));
        }

        clone_resources
    }
    /// gets a reference to the resource with index `ri`
    /// returns `None` if `ri` is invaild
    fn get(&self, ri: usize) -> Option<MutexGuard<Resource>> {
        if ri >= self.resources.len() {
            return None;
        }

        Some(self.resources[ri].lock())
    }
}
/// gets a resource with ri `ri` then executes then on it
pub fn get_resource<DO, R>(ri: usize, then: DO) -> Option<R>
where
    DO: FnOnce(MutexGuard<Resource>) -> R,
{
    super::with_current(|current| {
        current
            .state()
            .resource_manager()
            .unwrap()
            .get(ri)
            .filter(|r| !matches!(**r, Resource::Null))
            .map(then)
    })
}

/// adds a resource to the current process
pub fn add_resource(resource: Resource) -> usize {
    super::with_current(move |current| {
        current
            .state_mut()
            .resource_manager_mut()
            .unwrap()
            .add_resource(resource)
    })
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: usize) -> Option<()> {
    super::with_current(move |current| {
        current
            .state_mut()
            .resource_manager_mut()
            .unwrap()
            .remove_resource(ri)
    })
}

/// clones the resources of the current process
pub fn clone_resources() -> Vec<ResourceItem> {
    super::with_current(|current| {
        current
            .state()
            .resource_manager()
            .unwrap()
            .clone_resources()
    })
}
