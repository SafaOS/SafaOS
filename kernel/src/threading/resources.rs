use core::fmt::Debug;

use alloc::vec::Vec;

use crate::drivers::vfs::{DirIter, FileDescriptor, FS, VFS_STRUCT};

#[derive(Clone)]
pub enum Resource {
    Null,
    File(FileDescriptor),
    /// TODO: better diriter implementation
    DirIter(DirIter),
}

impl Resource {
    pub const fn variant(&self) -> u8 {
        match self {
            Resource::Null => 0,
            Resource::File(_) => 1,
            Resource::DirIter(_) => 2,
        }
    }
}

pub struct ResourceManager {
    resources: Vec<Resource>,
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
                    .filter(|(_, r)| !matches!(r, Resource::Null))
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

    pub fn add_resource(&mut self, resource: Resource) -> usize {
        let resources = &mut self.resources[self.next_ri..];

        for (mut ri, res) in resources.iter_mut().enumerate() {
            if res.variant() == Resource::Null.variant() {
                ri += self.next_ri;

                self.next_ri = ri;
                *res = resource;

                return ri;
            }
        }

        self.resources.push(resource);

        let ri = self.resources.len() - 1;
        self.next_ri = ri;

        ri
    }

    #[inline]
    pub fn remove_resource(&mut self, ri: usize) -> Result<(), ()> {
        if ri >= self.resources.len() {
            return Err(());
        }

        self.resources[ri] = Resource::Null;
        if ri < self.next_ri {
            self.next_ri = ri;
        }
        Ok(())
    }

    /// cleans up all resources
    /// returns the **previous** next resource index
    pub fn clean(&mut self) -> usize {
        for resource in &mut self.resources {
            match resource {
                Resource::File(fd) => VFS_STRUCT.read().close(fd).unwrap(),
                _ => *resource = Resource::Null,
            }
        }

        let prev = self.next_ri;
        self.next_ri = 0;
        prev
    }

    pub fn next_ri(&self) -> usize {
        self.next_ri
    }

    pub fn overwrite_resources(&mut self, resources: Vec<Resource>) {
        self.resources = resources;
    }

    pub fn clone_resources(&self) -> Vec<Resource> {
        self.resources.clone()
    }

    /// gets a mutable reference to the resource with index `ri`
    /// returns `None` if `ri` is invaild
    pub fn get(&mut self, ri: usize) -> Option<&mut Resource> {
        let resources = &mut self.resources;

        if ri >= resources.len() {
            return None;
        }

        Some(&mut resources[ri])
    }
}

// TODO: lock? or should every resource handle it's own lock?
/// gets a resource by `ri` and executes `then` on it
/// returns `None` if `ri` doesn't exist
/// returns `Some(R)` where `R` is the result of `then` if sucessful
pub fn with_resource<DO, R>(ri: usize, then: DO) -> Option<R>
where
    DO: FnOnce(&mut Resource) -> R,
{
    super::with_current_state(|state| state.resource_manager.get_mut().get(ri).map(then))
}

/// adds a resource to the current process
pub fn add_resource(resource: Resource) -> usize {
    super::with_current_state(move |state| state.resource_manager.lock().add_resource(resource))
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: usize) -> Result<(), ()> {
    super::with_current_state(move |state| state.resource_manager.lock().remove_resource(ri))
}
