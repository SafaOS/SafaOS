use crate::{drivers::vfs::CollectionIterDescriptor, process::resources, utils::locks::Mutex};

/// a wrapper around a DirIterDescriptor resource which closes the diriter when dropped
pub struct DirIter(pub(super) usize);

impl DirIter {
    /// Creates a new `DirIter` from a resource index.
    /// takes ownership of the resource index, meaning that the resource will be closed when the `DirIter` is dropped.
    pub fn from_ri(ri: usize) -> Option<Result<Self, ()>> {
        resources::get_ref(ri, |resource| {
            if let Some(_) = resource.data().as_ref::<Mutex<CollectionIterDescriptor>>() {
                Ok(Self(ri))
            } else {
                Err(())
            }
        })
    }
}

impl Drop for DirIter {
    fn drop(&mut self) {
        assert!(
            resources::remove_resource(self.0),
            "Failed to Drop a DirIter, invalid Resource ID"
        );
    }
}
