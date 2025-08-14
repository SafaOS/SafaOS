use core::sync::atomic::{AtomicUsize, Ordering};

use crate::drivers::vfs::{FSObjectID, FSObjectType, SeekOffset};
use crate::memory::page_allocator::PageAlloc;
use crate::utils::alloc::PageVec;
use crate::utils::locks::RwLock;
use crate::utils::path::PathParts;
use alloc::boxed::Box;
use alloc::vec::Vec;
use hashbrown::{DefaultHashBuilder, HashMap};
use safa_abi::fs::{DirEntry, FileAttr};

use crate::devices::{Device, DeviceInterface};

use super::FileName;
use super::{FSError, FSResult, FileSystem};

pub enum RamFSObjectState {
    Data(PageVec<u8>),
    Collection(HashMap<FileName, FSObjectID, DefaultHashBuilder, PageAlloc>),
    StaticDevice(&'static dyn Device),
    StaticInterface(&'static dyn DeviceInterface),
}

impl core::fmt::Debug for RamFSObjectState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Data(data) => write!(f, "Data({})", data.len()),
            Self::Collection(items) => write!(f, "Collection({items:?})"),
            Self::StaticDevice(device) => write!(f, "Device({})", device.name()),
            Self::StaticInterface(i) => write!(f, "Device({})", i.name()),
        }
    }
}

#[derive(Debug)]
pub struct RamFSObject {
    state: RamFSObjectState,
    /// the amount of times this object is referenced by another object (a collection)
    reference_count: AtomicUsize,
    /// the amount of handles (resources) opened on this object
    opened_handles: AtomicUsize,
}

impl RamFSObject {
    const fn kind(&self) -> FSObjectType {
        match self.state {
            RamFSObjectState::Data(_) => FSObjectType::File,
            RamFSObjectState::Collection(_) => FSObjectType::Directory,
            RamFSObjectState::StaticDevice(_) => FSObjectType::Device,
            RamFSObjectState::StaticInterface(_) => FSObjectType::Device,
        }
    }

    fn size(&self) -> usize {
        match self.state {
            RamFSObjectState::Data(ref data) => data.len(),
            RamFSObjectState::Collection(ref collection) => collection.len(),
            RamFSObjectState::StaticDevice(_) | RamFSObjectState::StaticInterface(_) => 0,
        }
    }

    fn direntry(&self, name: &str) -> DirEntry {
        let size = self.size();
        let kind = self.kind();
        DirEntry::new(name, FileAttr::new(kind, size))
    }

    fn attrs(&self) -> FileAttr {
        let size = self.size();
        let kind = self.kind();
        FileAttr::new(kind, size)
    }

    pub fn new(state: RamFSObjectState) -> Self {
        Self {
            state,
            opened_handles: AtomicUsize::new(0),
            reference_count: AtomicUsize::new(1),
        }
    }

    pub fn write(&mut self, offset: SeekOffset, buf: &[u8]) -> FSResult<usize> {
        fn write_data(data: &mut PageVec<u8>, offset: SeekOffset, buf: &[u8]) -> FSResult<usize> {
            let start = match offset {
                SeekOffset::Start(from_start) => from_start,
                SeekOffset::End(from_end) => data.len().saturating_sub(from_end),
            };

            if start > data.len() {
                return Err(FSError::InvalidOffset);
            }

            let data_len = data.len();
            let len = data_len - start;
            let buf_len = buf.len();

            if buf_len > len {
                data.resize((buf_len - len) + data_len, 0);
            }

            let write_len = buf_len;
            data[start..start + write_len].copy_from_slice(&buf[..write_len]);

            Ok(write_len)
        }

        match self.state {
            RamFSObjectState::Data(ref mut data) => write_data(data, offset, buf),
            RamFSObjectState::StaticDevice(device) => device.write(offset, buf),
            _ => Err(FSError::NotAFile),
        }
    }

    pub fn truncate(&mut self, size: usize) -> FSResult<()> {
        match self.state {
            RamFSObjectState::Data(ref mut data) => {
                data.truncate(size);
                Ok(())
            }
            _ => Err(FSError::NotAFile),
        }
    }

    pub fn sync(&self) -> FSResult<()> {
        match self.state {
            RamFSObjectState::Data(_) => Ok(()),
            RamFSObjectState::StaticDevice(device) => device.sync(),
            _ => Err(FSError::NotAFile),
        }
    }

    fn open_mmap_interface(
        &self,
        offset: SeekOffset,
        page_count: usize,
    ) -> FSResult<Box<dyn crate::process::vas::MemMappedInterface>> {
        match self.state {
            RamFSObjectState::StaticDevice(d) => d.mmap(offset, page_count),
            _ => Err(FSError::OperationNotSupported),
        }
    }

    fn send_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        match self.state {
            RamFSObjectState::StaticDevice(device) => device.send_command(cmd, arg),
            _ => Err(FSError::NotAFile),
        }
    }

    pub fn read(&self, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        match self.state {
            RamFSObjectState::Data(ref data) => {
                let start = match offset {
                    SeekOffset::Start(from_start) => from_start,
                    SeekOffset::End(from_end) => data.len().saturating_sub(from_end),
                };

                if start > data.len() {
                    return Err(FSError::InvalidOffset);
                }

                let len = buf.len().min(data.len() - start);
                buf[..len].copy_from_slice(&data[start..start + len]);

                Ok(len)
            }
            RamFSObjectState::StaticDevice(device) => device.read(offset, buf),
            _ => Err(FSError::NotAFile),
        }
    }

    pub fn add_child(&mut self, object_id: FSObjectID, name: FileName) -> FSResult<()> {
        match self.state {
            RamFSObjectState::Collection(ref mut collection) => collection
                .try_insert(name, object_id)
                .map(|_| ())
                .map_err(|_| FSError::AlreadyExists),
            _ => Err(FSError::NotADirectory),
        }
    }

    pub fn get_child(&self, name: &str) -> FSResult<FSObjectID> {
        match self.state {
            RamFSObjectState::Collection(ref collection) => {
                collection.get(name).copied().ok_or(FSError::NotFound)
            }
            _ => Err(FSError::NotADirectory),
        }
    }

    pub fn remove_child(&mut self, name: &str) -> FSResult<()> {
        match self.state {
            RamFSObjectState::Collection(ref mut collection) => {
                collection.remove(name);
                Ok(())
            }
            _ => Err(FSError::NotADirectory),
        }
    }

    pub fn get_children(&self) -> FSResult<(impl Iterator<Item = (&FileName, FSObjectID)>, usize)> {
        match self.state {
            RamFSObjectState::Collection(ref collection) => {
                let len = collection.len();
                Ok((collection.iter().map(|(key, value)| (key, *value)), len))
            }
            _ => Err(FSError::NotADirectory),
        }
    }

    pub fn is_non_empty_collection(&self) -> bool {
        match self.state {
            RamFSObjectState::Collection(ref items) => {
                // collection but empty
                if items.is_empty() {
                    return false;
                }
                // check if the collection contains only two items that is the pointer to the previous collection and the pointer to the current collection
                if items.len() == 2 {
                    // collection but empty
                    !(items.get("..").is_some() && items.get(".").is_some())
                } else {
                    // non-empty collection
                    true
                }
            }
            // not a collection
            _ => false,
        }
    }

    pub const fn is_collection(&self) -> bool {
        match self.state {
            RamFSObjectState::Collection(_) => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct RamFS {
    objects: HashMap<FSObjectID, RamFSObject>,
    next_id: FSObjectID,
}

impl RamFS {
    fn get(&self, id: FSObjectID) -> Option<&RamFSObject> {
        let obj = self.objects.get(&id);
        obj
    }

    fn get_mut(&mut self, id: FSObjectID) -> Option<&mut RamFSObject> {
        self.objects.get_mut(&id)
    }

    fn add_file(&mut self) -> FSResult<FSObjectID> {
        let id = self.next_id;
        self.next_id += 1;
        self.objects
            .insert(id, RamFSObject::new(RamFSObjectState::Data(PageVec::new())));
        Ok(id)
    }

    /// Create a new directory object (object collection) and returns its ID
    fn add_directory(&mut self) -> FSResult<FSObjectID> {
        let id = self.next_id;
        self.next_id += 1;

        let mut collection = HashMap::new_in(PageAlloc);
        // don't increase the reference count because these are going to be deleted when the directory is deleted, and the directory will be treated as empty
        let previous_dir_name = FileName::new_const("..");
        let current_dir_name = FileName::new_const(".");
        collection.insert(previous_dir_name, id);
        collection.insert(current_dir_name, id);

        let object = RamFSObject::new(RamFSObjectState::Collection(collection));

        self.objects.insert(id, object);
        Ok(id)
    }

    /// Create a new device object and returns its ID
    fn add_device(&mut self, device: &'static dyn Device) -> FSResult<FSObjectID> {
        let id = self.next_id;
        self.next_id += 1;

        let object = RamFSObject::new(RamFSObjectState::StaticDevice(device));

        self.objects.insert(id, object);
        Ok(id)
    }

    /// Create a new device object and returns its ID
    fn add_device_interface(
        &mut self,
        interface: &'static dyn DeviceInterface,
    ) -> FSResult<FSObjectID> {
        let id = self.next_id;
        self.next_id += 1;

        let object = RamFSObject::new(RamFSObjectState::StaticInterface(interface));

        self.objects.insert(id, object);
        Ok(id)
    }

    /// Get a reference to a file object, returns an error if the object does not exist, unlike `get` which returns None
    fn fget(&self, id: FSObjectID) -> FSResult<&RamFSObject> {
        self.get(id).ok_or(FSError::NotFound)
    }

    /// Get a mutable reference to a file object, returns an error if the object does not exist, unlike `get_mut` which returns None
    fn fget_mut(&mut self, id: FSObjectID) -> FSResult<&mut RamFSObject> {
        self.get_mut(id).ok_or(FSError::NotFound)
    }
}

impl RamFS {
    pub fn create() -> Self {
        let root_collection = HashMap::new_in(PageAlloc);
        let root_obj = RamFSObject::new(RamFSObjectState::Collection(root_collection));
        let mut objects = HashMap::new();
        objects.insert(0, root_obj);

        Self {
            next_id: 1,
            objects,
        }
    }

    fn read(&self, id: FSObjectID, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        let obj = self.fget(id)?;
        let results = obj.read(offset, buf);
        // FIXME: if the object is a pointer and the pointed object is deleted it could be a problem...
        results
    }

    fn write(&mut self, id: FSObjectID, offset: SeekOffset, data: &[u8]) -> FSResult<usize> {
        let obj = self.fget_mut(id)?;
        let results = obj.write(offset, data);
        // FIXME: if the object is a pointer and the pointed object is deleted it could be a problem...
        results
    }

    fn truncate(&mut self, id: FSObjectID, size: usize) -> FSResult<()> {
        let obj = self.fget_mut(id)?;
        let results = obj.truncate(size);
        // FIXME: if the object is a pointer and the pointed object is deleted it could be a problem...
        results
    }

    fn sync(&self, id: FSObjectID) -> FSResult<()> {
        let obj = self.fget(id)?;
        let results = obj.sync();
        // FIXME: if the object is a pointer and the pointed object is deleted it could be a problem...
        results
    }

    fn create_file(&mut self, parent_id: FSObjectID, name: &str) -> FSResult<FSObjectID> {
        let name = FileName::try_from(name).map_err(|_| FSError::InvalidName)?;

        // ensures that the parent exists and is a collection (directory)
        let parent = self.fget(parent_id)?;
        if !parent.is_collection() {
            return Err(FSError::NotADirectory);
        }

        // actually create the file
        let created_id = self.add_file()?;
        let parent = self.fget_mut(parent_id)?;

        // append the child to the parent's children collection
        parent.add_child(created_id, name)?;

        Ok(created_id)
    }

    fn create_directory(&mut self, parent_id: FSObjectID, name: &str) -> FSResult<FSObjectID> {
        let name = FileName::try_from(name).map_err(|_| FSError::InvalidName)?;

        // ensures that the parent exists and is a collection (directory)
        let parent = self.fget(parent_id)?;
        if !parent.is_collection() {
            return Err(FSError::NotADirectory);
        }

        // actually create the directory
        let created_id = self.add_directory()?;
        let parent = self.fget_mut(parent_id)?;

        // append the child to the parent's children collection
        parent.add_child(created_id, name)?;

        Ok(created_id)
    }

    fn create_device(
        &mut self,
        parent_id: FSObjectID,
        name: &str,
        device: &'static dyn Device,
    ) -> FSResult<FSObjectID> {
        let name = FileName::try_from(name).map_err(|_| FSError::InvalidName)?;

        // ensures that the parent exists and is a collection (directory)
        let parent = self.fget(parent_id)?;
        if !parent.is_collection() {
            return Err(FSError::NotADirectory);
        }

        // actually create the directory
        let created_id = self.add_device(device)?;
        let parent = self.fget_mut(parent_id)?;

        // append the child to the parent's children collection
        parent.add_child(created_id, name)?;

        Ok(created_id)
    }

    fn create_device_interface(
        &mut self,
        parent_id: FSObjectID,
        name: &str,
        interface: &'static dyn DeviceInterface,
    ) -> FSResult<FSObjectID> {
        let name = FileName::try_from(name).map_err(|_| FSError::InvalidName)?;

        // ensures that the parent exists and is a collection (directory)
        let parent = self.fget(parent_id)?;
        if !parent.is_collection() {
            return Err(FSError::NotADirectory);
        }

        // actually create the directory
        let created_id = self.add_device_interface(interface)?;
        let parent = self.fget_mut(parent_id)?;

        // append the child to the parent's children collection
        parent.add_child(created_id, name)?;

        Ok(created_id)
    }

    fn remove(
        &mut self,
        child_name: &str,
        parent_id: FSObjectID,
        child_id: FSObjectID,
    ) -> FSResult<()> {
        let obj = self.fget(child_id)?;
        // makes sure the object is a non-empty collection if it is a collection, this is for
        // to prevent accidental deletion of non-empty directories and to make sure the directory is deleted recursively by the software (to delete hardlinks and such)
        if obj.is_non_empty_collection() {
            return Err(FSError::DirectoryNotEmpty);
        }

        let parent = self.fget_mut(parent_id)?;
        parent.remove_child(child_name)?;

        let obj = self.fget_mut(child_id)?;
        *obj.reference_count.get_mut() -= 1;

        Ok(())
    }

    // closes the object returns whether or not there is any more open handles and if there is any more references to it
    fn close(&self, id: FSObjectID) -> (bool, bool) {
        let obj = self.get(id).expect("attempt to close a non-existent ID");
        (
            obj.opened_handles.fetch_sub(1, Ordering::Relaxed) == 0,
            obj.reference_count.load(Ordering::Relaxed) == 0,
        )
    }
}

impl FileSystem for RwLock<RamFS> {
    fn name(&self) -> &'static str {
        "RamFS"
    }

    fn read(&self, id: FSObjectID, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        self.read().read(id, offset, buf)
    }

    fn write(&self, id: FSObjectID, offset: SeekOffset, data: &[u8]) -> FSResult<usize> {
        self.write().write(id, offset, data)
    }

    fn sync(&self, id: FSObjectID) -> FSResult<()> {
        self.read().sync(id)
    }
    fn send_command(&self, id: FSObjectID, cmd: u16, arg: u64) -> FSResult<()> {
        let read_guard = self.read();
        let obj = read_guard.fget(id)?;
        obj.send_command(cmd, arg)
    }

    fn truncate(&self, id: FSObjectID, size: usize) -> FSResult<()> {
        self.write().truncate(id, size)
    }

    fn create_file(&self, parent_id: FSObjectID, name: &str) -> FSResult<FSObjectID> {
        self.write().create_file(parent_id, name)
    }

    fn create_directory(&self, parent_id: FSObjectID, name: &str) -> FSResult<FSObjectID> {
        self.write().create_directory(parent_id, name)
    }

    fn mount_device(
        &self,
        parent_id: FSObjectID,
        name: &str,
        device: &'static dyn Device,
    ) -> FSResult<FSObjectID> {
        self.write().create_device(parent_id, name, device)
    }

    fn mount_device_interface(
        &self,
        parent_id: FSObjectID,
        name: &str,
        interface: &'static dyn DeviceInterface,
    ) -> FSResult<FSObjectID> {
        self.write()
            .create_device_interface(parent_id, name, interface)
    }

    fn resolve_path_rel(&self, parent_id: FSObjectID, path: PathParts) -> FSResult<FSObjectID> {
        super::resolve_path_parts(parent_id, path, |_, parent_id, obj_name| {
            let read_guard = self.read();
            let parent = read_guard.fget(parent_id)?;
            parent.get_child(obj_name)
        })
    }

    fn on_open(&self, id: FSObjectID) -> FSResult<Option<Box<dyn Device>>> {
        let read_guard = self.read();
        let obj = read_guard.fget(id)?;
        obj.opened_handles.fetch_add(1, Ordering::Relaxed);
        let results = match obj.state {
            RamFSObjectState::StaticInterface(i) => Some(i.open()),
            _ => None,
        };
        Ok(results)
    }

    fn remove(&self, child_name: &str, parent_id: FSObjectID, id: FSObjectID) -> FSResult<()> {
        self.write().remove(child_name, parent_id, id)
    }

    fn on_close(&self, id: FSObjectID) -> FSResult<()> {
        let read_guard = self.read();
        let (is_last_opened, no_references) = read_guard.close(id);
        if is_last_opened && no_references {
            drop(read_guard);
            let mut write_guard = self.write();
            write_guard.objects.remove(&id);
        }

        Ok(())
    }

    fn get_children(&self, id: FSObjectID) -> FSResult<Box<[DirEntry]>> {
        let read_guard = self.read();
        let obj = read_guard.fget(id)?;

        let (children_iter, children_count) = obj.get_children()?;
        let mut children = Vec::with_capacity(children_count);

        for (name, child_id) in children_iter {
            // fget should dereference the pointer to get the object
            let child_obj = read_guard.fget(child_id)?;
            children.push(child_obj.direntry(name));
        }

        Ok(children.into_boxed_slice())
    }

    fn attrs_of(&self, id: FSObjectID) -> safa_abi::fs::FileAttr {
        let read_guard = self.read();
        let obj = read_guard.fget(id).expect("Object not found in the ramfs");
        obj.attrs()
    }

    fn open_mmap_interface(
        &self,
        id: FSObjectID,
        offset: SeekOffset,
        page_count: usize,
    ) -> FSResult<Box<dyn crate::process::vas::MemMappedInterface>> {
        let read_guard = self.read();
        let obj = read_guard.fget(id).expect("Object not found in the ramfs");
        obj.open_mmap_interface(offset, page_count)
    }
}
