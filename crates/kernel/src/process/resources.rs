use core::{fmt::Debug, sync::atomic::AtomicBool};

use crate::{
    drivers::vfs::{CollectionIterDescriptor, FSResult},
    process::{self, vas::TrackedMemoryMapping},
    sockets::{ServerSocketDesc, SocketClientConn, SocketDomain, SocketKind, SocketServerConn},
    utils::locks::Mutex,
};
use alloc::sync::Arc;
use hashbrown::HashMap;
use safa_abi::errors::ErrorStatus;

use crate::drivers::vfs::FSObjectDescriptor;

/// A resource ID
pub type Ri = usize;

pub enum ResourceData {
    File(FSObjectDescriptor),
    DirIter(Mutex<CollectionIterDescriptor>),
    TrackedMapping(TrackedMemoryMapping),
    SocketDesc {
        domain: SocketDomain,
        kind: SocketKind,
        can_block: bool,
    },
    ServerSocket(ServerSocketDesc),
    ServerSocketConn(SocketServerConn),
    ClientSocketConn(SocketClientConn),
}

impl ResourceData {
    pub fn try_clone(&self) -> Result<Self, ()> {
        match self {
            Self::File(file) => Ok(Self::File(file.clone())),
            Self::DirIter(coll) => Ok(Self::DirIter(Mutex::new(coll.lock().clone()))),
            Self::SocketDesc {
                domain,
                kind,
                can_block,
            } => Ok(Self::SocketDesc {
                domain: *domain,
                kind: *kind,
                can_block: *can_block,
            }),
            Self::ServerSocket(_)
            | Self::ClientSocketConn(_)
            | Self::ServerSocketConn(_)
            | Self::TrackedMapping(_) => Err(()),
        }
    }

    pub fn cloneable_to_different_address_space(&self) -> bool {
        match self {
            Self::TrackedMapping(_) => false,
            _ => true,
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

    pub fn try_clone(&self) -> Result<Self, ()> {
        Ok(Self {
            data: self.data.try_clone()?,
            global: AtomicBool::new(self.global.load(core::sync::atomic::Ordering::Acquire)),
        })
    }

    pub fn data(&self) -> &ResourceData {
        &self.data
    }

    pub fn cloneable_to_different_address_space(&self) -> bool {
        self.data.cloneable_to_different_address_space()
            && self.global.load(core::sync::atomic::Ordering::Acquire)
    }

    /// Performs a Sync operation on this resource
    /// # Safety
    /// Must be called from the address space owning this resource
    pub unsafe fn sync(&self) -> FSResult<()> {
        match self.data() {
            ResourceData::File(f) => f.sync(),
            ResourceData::TrackedMapping(m) => unsafe { m.sync().map(|_| ()) },
            ResourceData::DirIter(_)
            | ResourceData::ServerSocket(_)
            | ResourceData::ClientSocketConn(_)
            | ResourceData::ServerSocketConn(_)
            | ResourceData::SocketDesc { .. } => {
                Err(crate::drivers::vfs::FSError::OperationNotSupported)
            }
        }
    }
}

pub struct ResourceManager {
    resources: HashMap<Ri, Arc<Resource>>,
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
        self.resources.insert(ri, Arc::new(resource));
        self.next_resource_id += 1;
        ri
    }

    pub fn add_global_resource(&mut self, data: ResourceData) -> Ri {
        self.add_resource(Resource::new_global(data))
    }

    pub fn add_local_resource(&mut self, data: ResourceData) -> Ri {
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

    pub fn clone_resource(&mut self, ri: Ri) -> Option<Result<Resource, ()>> {
        let resource = self.get_mut(ri)?;
        // Only clones global resources
        if !resource.global.load(core::sync::atomic::Ordering::Acquire) {
            return None;
        }

        Some(resource.try_clone())
    }

    pub fn clone(&self) -> Self {
        let mut resources = HashMap::with_capacity(self.resources.capacity());
        for (res_id, res) in self.resources.iter() {
            if res.cloneable_to_different_address_space()
                && let Ok(res) = res.try_clone()
            {
                resources.insert(*res_id, Arc::new(res));
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
                new_resources.insert(resource_id, Arc::new(result));

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
    fn get<'s>(&'s self, ri: Ri) -> Option<&'s Arc<Resource>> {
        self.resources.get(&ri)
    }

    fn get_mut(&mut self, ri: Ri) -> Option<&mut Arc<Resource>> {
        self.resources.get_mut(&ri)
    }
}
// TODO: fgure out a better way to do this, where it's easier to tell that we are holding a lock on
// the current process state.

pub fn get_resource<DO, R, E: Into<ErrorStatus>>(ri: Ri, then: DO) -> Result<R, ErrorStatus>
where
    DO: FnOnce(Arc<Resource>) -> Result<R, E>,
{
    let res = {
        let this = process::current();
        this.resources()
            .get(ri)
            .cloned()
            .ok_or(ErrorStatus::UnknownResource)?
    };

    then(res).map_err(|e| e.into())
}

/// Gets a reference to resource with ri `ri` then executes then on it
///
/// If you are going to do something poteinally blocking use [get_resource] instead
pub fn get_resource_reference<DO, R>(ri: Ri, then: DO) -> Option<R>
where
    DO: FnOnce(&Resource) -> R,
{
    let this = process::current();
    this.resources_mut().get(ri).map(|r| then(r))
}

/// gets a resource with ri `ri` then executes then on it
pub fn get_resource_mut<DO, R>(ri: Ri, then: DO) -> Option<R>
where
    DO: FnOnce(&mut Arc<Resource>) -> R,
{
    let this = process::current();
    this.resources_mut().get_mut(ri).map(then)
}

/// Adds a resource that lives as long as the current process, to the current process
pub fn add_global_resource(resource_data: ResourceData) -> Ri {
    let this = process::current();
    this.resources_mut().add_global_resource(resource_data)
}

/// Duplicates a resource return the new duplicate resource's ID or None if that resource doesn't exist
pub fn duplicate_resource(ri: Ri) -> Option<Result<Ri, ()>> {
    let current_process = process::current();
    let mut manager = current_process.resources_mut();

    let resource = manager.clone_resource(ri)?;
    if let Err(()) = resource {
        return Some(Err(()));
    }

    Some(Ok(manager.add_resource(resource.unwrap())))
}

/// removes a resource from the current process with `ri`
pub fn remove_resource(ri: Ri) -> bool {
    let current_process = process::current();
    current_process.resources_mut().remove_resource(ri)
}
