mod cpuinfo;
mod eve_journal;
mod generic_file;
mod init_system;
mod kernelinfo;
mod meminfo;
mod processes;
mod usbinfo;

use self::{generic_file::GenericRodFSFile, init_system::InitStateItem};
use crate::{
    drivers::vfs::{FSError, FSObjectID, FSResult, FileSystem, SeekOffset},
    process::Pid,
    scheduler,
    utils::{alloc::PageVec, locks::RwLock},
};
use alloc::{boxed::Box, vec::Vec};
use hashbrown::HashMap;
use safa_abi::raw::io::{DirEntry, FSObjectType, FileAttr};

type OpaqueRodFSObjID = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ProcessObjID {
    inner_obj_id: OpaqueRodFSObjID,
    process_pid: Pid,
}

impl ProcessObjID {
    pub const fn new(inner_obj_id: OpaqueRodFSObjID, process_pid: Pid) -> Self {
        ProcessObjID {
            inner_obj_id,
            process_pid,
        }
    }

    pub const fn process_pid(&self) -> Pid {
        self.process_pid
    }

    pub const fn opaque_id(&self) -> OpaqueRodFSObjID {
        self.inner_obj_id
    }

    pub const fn from_obj_id(id: FSObjectID) -> Self {
        let bit_offset = size_of::<FSObjectID>() * 8 - 1;
        let process_mask = 1 << bit_offset;
        let id = id & !(process_mask);

        unsafe { core::mem::transmute(id) }
    }

    pub const fn to_obj_id(&self) -> FSObjectID {
        let bit_offset = size_of::<FSObjectID>() * 8 - 1;
        let process_mask = 1 << bit_offset;

        unsafe { core::mem::transmute::<ProcessObjID, FSObjectID>(*self) | process_mask }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Represents the identifier of a proc file system object.
/// either a process directory or any other object.
enum RodFSObjID {
    ProcessID(ProcessObjID),
    OtherID(OpaqueRodFSObjID),
}

impl RodFSObjID {
    pub const fn from_obj_id(id: FSObjectID) -> Self {
        let bit_offset = size_of::<FSObjectID>() * 8 - 1;
        let process_mask = 1 << bit_offset;
        if id & process_mask != 0 {
            let process_id = ProcessObjID::from_obj_id(id);
            RodFSObjID::ProcessID(process_id)
        } else {
            RodFSObjID::OtherID(id as u32)
        }
    }

    pub const fn to_obj_id(&self) -> FSObjectID {
        match self {
            RodFSObjID::ProcessID(id) => id.to_obj_id(),
            RodFSObjID::OtherID(id) => *id as usize,
        }
    }

    pub const fn opaque_id(&self) -> OpaqueRodFSObjID {
        match self {
            RodFSObjID::ProcessID(id) => id.opaque_id(),
            RodFSObjID::OtherID(id) => *id,
        }
    }
}

#[derive(Debug)]
enum RodFSObject {
    Collection {
        name: &'static str,
        /// Number of children including children of children
        size: u32,
    },
    File {
        inner: GenericRodFSFile,
        opened_handles: usize,
    },
}

impl RodFSObject {
    pub const fn new_collection(name: &'static str, size: u32) -> Self {
        RodFSObject::Collection { name, size }
    }

    pub const fn new_file(inner: GenericRodFSFile) -> Self {
        RodFSObject::File {
            inner,
            opened_handles: 0,
        }
    }

    pub const fn kind(&self) -> FSObjectType {
        match self {
            RodFSObject::Collection { .. } => FSObjectType::Directory,
            RodFSObject::File { .. } => FSObjectType::File,
        }
    }

    pub const fn is_collection(&self) -> bool {
        match self {
            RodFSObject::Collection { .. } => true,
            RodFSObject::File { .. } => false,
        }
    }

    pub const fn size(&self) -> u32 {
        match self {
            RodFSObject::Collection { size, .. } => *size + 1,
            RodFSObject::File { .. } => 1,
        }
    }

    pub const fn attrs(&self) -> FileAttr {
        FileAttr::new(self.kind(), 0)
    }

    pub fn name(&self) -> &'static str {
        match self {
            RodFSObject::Collection { name, .. } => name,
            RodFSObject::File { inner: file, .. } => file.name(),
        }
    }

    pub fn read(&mut self, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        match self {
            RodFSObject::File { inner, .. } => {
                let data = inner.get_data();
                let data_len = data.len();
                let offset = match offset {
                    SeekOffset::Start(pos) => pos,
                    SeekOffset::End(pos) => data_len - pos,
                };

                let len = data_len - offset;
                let len = len.min(buf.len());

                buf[..len].copy_from_slice(&data.as_bytes()[offset..offset + len]);
                Ok(len)
            }
            _ => Err(FSError::NotAFile),
        }
    }

    pub fn on_open(&mut self) {
        match self {
            RodFSObject::File { opened_handles, .. } => *opened_handles += 1,
            _ => {}
        }
    }

    pub fn close(&mut self) {
        match self {
            RodFSObject::File {
                inner,
                opened_handles,
            } => {
                *opened_handles -= 1;
                if *opened_handles == 0 {
                    inner.close();
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
/// The procfs is flat, an identifier is used as an index into the internal subsystem.
/// then using the index we search for a child using the length of the parent collection
/// which means doing things like creating a new file in an existing directory is impossible
pub struct InternalStructure {
    inner: PageVec<RodFSObject>,
}

impl InternalStructure {
    pub fn new() -> Self {
        let mut inner = PageVec::with_capacity(1);
        inner.push(RodFSObject::new_collection("", 0));

        InternalStructure { inner }
    }

    fn get_root_node_mut(&mut self) -> &mut RodFSObject {
        self.inner.first_mut().expect("procfs root node not found")
    }

    fn append_child(&mut self, child: RodFSObject) -> OpaqueRodFSObjID {
        let size = child.size();
        match self.get_root_node_mut() {
            RodFSObject::Collection {
                size: root_size, ..
            } => {
                *root_size += size;
            }
            _ => unreachable!("procfs root node is not a collection"),
        }
        self.inner.push(child);
        self.inner.len() as OpaqueRodFSObjID - 1
    }

    fn generate_from_init_state<const N: usize>(init_state: [InitStateItem; N]) -> Self {
        let mut structure = Self::new();
        for item in init_state {
            match item {
                InitStateItem::File(generic_file) => {
                    structure.append_child(RodFSObject::new_file(generic_file));
                }
            }
        }

        structure
    }

    fn generate_from_kernel() -> (Self, OpaqueRodFSObjID) {
        let init_state = const { init_system::get_init_state() };
        let mut results = Self::generate_from_init_state(init_state);
        let processes_collection_id = results.append_child(RodFSObject::Collection {
            name: "proc",
            size: 0,
        });
        (results, processes_collection_id)
    }

    fn generate_from_process(process_pid: Pid) -> Self {
        let init_state = init_system::process_init_system(process_pid);
        Self::generate_from_init_state(init_state)
    }

    fn try_generate_from_process(process_pid: Pid) -> Option<Self> {
        match scheduler::find(|process| process.pid() == process_pid, |_| ()) {
            Some(_) => Some(Self::generate_from_process(process_pid)),
            _ => None,
        }
    }

    fn get_mut(&mut self, idx: OpaqueRodFSObjID) -> Option<&mut RodFSObject> {
        self.inner.get_mut(idx as usize)
    }

    fn get_children(&mut self, idx: OpaqueRodFSObjID) -> FSResult<&mut [RodFSObject]> {
        let root_obj = self
            .inner
            .get(idx as usize)
            .expect("RodFS object ID invalid");

        if !root_obj.is_collection() {
            return Err(FSError::NotADirectory);
        }
        let size = root_obj.size();

        Ok(&mut self.inner[idx as usize + 1..idx as usize + size as usize])
    }

    pub fn search_indx(
        &self,
        start_id: OpaqueRodFSObjID,
        name: &str,
    ) -> FSResult<OpaqueRodFSObjID> {
        let mut obj_iter = self.inner.iter().skip(start_id as usize);
        let collection_obj = obj_iter
            .next()
            .expect("RodFS: InternalStructure invalid index passed to `search_indx`");

        let RodFSObject::Collection {
            size: collection_size,
            ..
        } = collection_obj
        else {
            return Err(FSError::NotADirectory);
        };

        let mut index = start_id + 1;

        while let Some(obj) = obj_iter.next()
            && index <= (collection_size + start_id)
        {
            match obj {
                // if found
                _ if obj.name() == name => return Ok(index),
                // if it is a collection, add its size + 1 (its header)
                RodFSObject::Collection { size, .. } => {
                    let _ = obj_iter.by_ref().take(*size as usize);
                    index += size + 1;
                }
                RodFSObject::File { .. } => index += 1,
            }
        }

        Err(FSError::NotFound)
    }
}

pub struct RodFS {
    internal_structure: InternalStructure,
    /// processes however are given special treatment as they are not part of the internal system, same search mechanism is used for processes as well
    processes_cache: HashMap<u32, InternalStructure>,
    processes_collection_id: OpaqueRodFSObjID,
}

impl RodFS {
    pub fn create() -> Self {
        let (internal_structure, processes_collection_id) =
            InternalStructure::generate_from_kernel();
        RodFS {
            internal_structure,
            processes_cache: HashMap::new(),
            processes_collection_id,
        }
    }

    const fn processes_collection_id(&self) -> RodFSObjID {
        RodFSObjID::OtherID(self.processes_collection_id)
    }

    fn get_internal(&mut self, obj_id: RodFSObjID) -> Option<&mut InternalStructure> {
        match obj_id {
            RodFSObjID::OtherID(_) => Some(&mut self.internal_structure),
            RodFSObjID::ProcessID(process_obj_id) => {
                let process_pid = process_obj_id.process_pid();
                // TODO: there is a better solution but the borrow checker is not happy with it, even tho it looks working to me
                // i might be sleepy or something, check after bumping nightly
                if self.processes_cache.contains_key(&process_pid) {
                    return self.processes_cache.get_mut(&process_pid);
                }

                self.processes_cache.insert(
                    process_pid,
                    InternalStructure::try_generate_from_process(process_pid)?,
                );
                self.processes_cache.get_mut(&process_pid)
            }
        }
    }

    fn get(&mut self, obj_id: RodFSObjID) -> Option<&mut RodFSObject> {
        self.get_internal(obj_id)
            .and_then(|structure| structure.get_mut(obj_id.opaque_id()))
    }

    fn get_children(&mut self, obj_id: RodFSObjID) -> FSResult<&mut [RodFSObject]> {
        self.get_internal(obj_id)
            .map(|structure| structure.get_children(obj_id.opaque_id()))
            .expect("invalid obj_id")
    }

    fn search_indx(&mut self, parent_id: RodFSObjID, name: &str) -> FSResult<RodFSObjID> {
        if parent_id == self.processes_collection_id() {
            if name == "self" {
                let curr_process = scheduler::this_process();

                let pid = curr_process.pid();
                let process_obj_id = ProcessObjID::new(0, pid);
                return Ok(RodFSObjID::ProcessID(process_obj_id));
            }

            if let Ok(pid) = name.parse::<u32>() {
                let process_obj_id = ProcessObjID::new(0, pid);
                return Ok(RodFSObjID::ProcessID(process_obj_id));
            }
        }

        let structure = self.get_internal(parent_id).ok_or(FSError::NotFound)?;

        let parent_opaque_id = parent_id.opaque_id();
        let opaque_id = structure.search_indx(parent_opaque_id, name)?;

        match parent_id {
            RodFSObjID::OtherID(_) => Ok(RodFSObjID::OtherID(opaque_id)),
            RodFSObjID::ProcessID(process_obj_id) => {
                let process_pid = process_obj_id.process_pid();
                Ok(RodFSObjID::ProcessID(ProcessObjID::new(
                    opaque_id,
                    process_pid,
                )))
            }
        }
    }

    fn cleanup(&mut self, obj_id: RodFSObjID) {
        match obj_id {
            RodFSObjID::OtherID(_) => {}
            RodFSObjID::ProcessID(process_obj_id) => {
                let process_pid = process_obj_id.process_pid();
                // if the process is not found, it means the process has been terminated
                // remove the process from the cache
                if scheduler::find(|process| process.pid() == process_pid, |_| ()).is_none() {
                    self.processes_cache.remove(&process_pid);
                }
            }
        }
    }
}

impl FileSystem for RwLock<RodFS> {
    fn name(&self) -> &'static str {
        "RodFS"
    }

    fn on_open(&self, id: FSObjectID) -> FSResult<()> {
        let mut write_guard = self.write();
        let obj_id = RodFSObjID::from_obj_id(id);
        let obj = write_guard.get(obj_id).expect("invalid RodFS Object ID");
        obj.on_open();
        Ok(())
    }

    fn on_close(&self, id: FSObjectID) -> FSResult<()> {
        let mut write_guard = self.write();

        let obj_id = RodFSObjID::from_obj_id(id);
        let obj = write_guard.get(obj_id).expect("invalid RodFS Object ID");

        obj.close();
        write_guard.cleanup(obj_id);
        Ok(())
    }

    fn read(&self, id: FSObjectID, offset: SeekOffset, buf: &mut [u8]) -> FSResult<usize> {
        let mut write_guard = self.write();
        let obj_id = RodFSObjID::from_obj_id(id);

        let obj = write_guard.get(obj_id).expect("invalid RodFS Object ID");
        obj.read(offset, buf)
    }

    fn attrs_of(&self, id: FSObjectID) -> FileAttr {
        let obj_id = RodFSObjID::from_obj_id(id);
        let mut write_guard = self.write();
        let obj = write_guard.get(obj_id).expect("invalid RodFS Object ID");
        obj.attrs()
    }

    fn resolve_path_rel(
        &self,
        parent_id: FSObjectID,
        path: crate::utils::path::PathParts,
    ) -> FSResult<FSObjectID> {
        super::resolve_path_parts(parent_id, path, |_, parent_id, name| {
            let parent_obj_id = RodFSObjID::from_obj_id(parent_id);
            self.write()
                .search_indx(parent_obj_id, name)
                .map(|obj_id| obj_id.to_obj_id())
        })
    }

    fn get_children(&self, id: FSObjectID) -> FSResult<Box<[DirEntry]>> {
        let mut write_guard = self.write();
        let obj_id = RodFSObjID::from_obj_id(id);
        let children_raw = write_guard.get_children(obj_id)?;

        let len = children_raw.len();

        let mut children = Vec::with_capacity(len);

        for child_raw in children_raw {
            let attrs = child_raw.attrs();
            let name = child_raw.name();
            let direntry = DirEntry::new(name, attrs);

            children.push(direntry);
        }

        // FIXME: Could be made simpler
        if obj_id == write_guard.processes_collection_id() {
            use core::fmt::Write;

            let mut pid_fmt_buf = heapless::String::<20>::new();
            scheduler::for_each(|process| {
                _ = write!(pid_fmt_buf, "{}", process.pid());
                let attrs = FileAttr::new(FSObjectType::Directory, 0);

                let direntry = DirEntry::new(&pid_fmt_buf, attrs);
                children.push(direntry);

                pid_fmt_buf.clear();
            });

            let attrs = FileAttr::new(FSObjectType::Directory, 0);
            let direntry = DirEntry::new("self", attrs);
            children.push(direntry);
        }

        Ok(children.into_boxed_slice())
    }
}
