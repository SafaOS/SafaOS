use core::{fmt::Debug, sync::atomic::AtomicBool};

use crate::{drivers::vfs::CollectionIterDescriptor, process, thread, utils::locks::Mutex};
use hashbrown::HashMap;

use crate::drivers::vfs::FSObjectDescriptor;

/// A resource ID
pub type Ri = usize;

pub enum ResourceData {
    File(FSObjectDescriptor),
    DirIter(Mutex<CollectionIterDescriptor>),
}

impl ResourceData {
    pub fn clone(&mut self) -> Self {
        match self {
            Self::File(file) => Self::File(file.clone()),
            Self::DirIter(coll) => Self::DirIter(Mutex::new(coll.get_mut().clone())),
        }
    }
}

pub struct Resource {
    data: ResourceData,
    /// Whether or not the Resource is tracked by a single thread or the entire process
    /// likely true
    global: AtomicBool,
}

impl Resource {
    pub const fn new(data: ResourceData, is_global: bool) -> Self {
        Self {
            data,
            global: AtomicBool::new(is_global),
        }
    }

    pub const fn new_global(data: ResourceData) -> Self {
        Self::new(data, true)
    }

    pub const fn new_local(data: ResourceData) -> Self {
        Self::new(data, false)
    }

    pub fn clone(&mut self) -> Self {
        Self {
            data: self.data.clone(),
            global: AtomicBool::new(*self.global.get_mut()),
        }
    }

    pub fn data(&self) -> &ResourceData {
        &self.data
    }
}

pub struct ResourceManager {
    resources: HashMap<Ri, Resource>,
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

    fn add_resource(&mut self, resource: Resource) -> Ri {
        let ri = self.next_resource_id;
        self.resources.insert(ri, resource);
        self.next_resource_id += 1;
        ri
    }

    pub fn add_global_resource(&mut self, data: ResourceData) -> Ri {
        self.add_resource(Resource::new_global(data))
    }

    fn add_local_resource(&mut self, data: ResourceData) -> Ri {
        self.add_resource(Resource::new_local(data))
    }

    #[inline]
    pub fn remove_resource(&mut self, ri: Ri) -> bool {
        // TODO: keep track of resource ids
        match self.resources.remove(&ri) {
            None => false,
            Some(_) => true,
        }
    }

    pub fn overwrite_resources(&mut self, resources: Self) {
        *self = resources;
    }

    pub fn clone_resource(&mut self, ri: Ri) -> Option<Resource> {
        let resource = self.get_mut(ri)?;
        Some(resource.clone())
    }

    pub fn clone(&mut self) -> Self {
        let mut resources = HashMap::with_capacity(self.resources.capacity());
        for (res_id, res) in self.resources.iter_mut() {
            resources.insert(*res_id, res.clone());
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
            new_resources.insert(resource_id, result);

            if max_resource_id < resource_id {
                max_resource_id = resource_id;
            }
        }

        Ok(Self {
            resources: new_resources,
            next_resource_id: max_resource_id + 1,
        })
    }

    /// gets a reference to the resource with index `ri`
    /// returns `None` if `ri` is invalid
    fn get<'s>(&'s self, ri: Ri) -> Option<&'s Resource> {
        self.resources.get(&ri)
    }

    fn get_mut(&mut self, ri: Ri) -> Option<&mut Resource> {
        self.resources.get_mut(&ri)
    }
}
// TODO: fgure out a better way to do this, where it's easier to tell that we are holding a lock on
// the current process state.

/// gets a resource with ri `ri` then executes then on it
pub fn get_resource<DO, R>(ri: Ri, then: DO) -> Option<R>
where
    DO: FnOnce(&Resource) -> R,
{
    let this = process::current();
    this.resources().get(ri).map(then)
}

/// Adds a resource that lives as long as the current process, to the current process
pub fn add_global_resource(resource_data: ResourceData) -> Ri {
    let this = process::current();
    this.resources_mut().add_global_resource(resource_data)
}

/// Adds a resource that lives as long as the current thread, to the current process
pub fn add_local_resource(resource_data: ResourceData) -> Ri {
    let curr_thread = thread::current();
    let curr_proc = curr_thread.process();
    let ri = curr_proc.resources_mut().add_local_resource(resource_data);
    curr_thread.take_resource(ri);
    ri
}

/// Duplicates a resource return the new duplicate resource's ID or None if that resource doesn't exist
pub fn duplicate_resource(ri: Ri) -> Option<Ri> {
    let current_process = process::current();
    let mut manager = current_process.resources_mut();

    let resource = manager.clone_resource(ri)?;
    Some(manager.add_resource(resource))
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: Ri) -> bool {
    let current_process = process::current();
    current_process.resources_mut().remove_resource(ri)
}
