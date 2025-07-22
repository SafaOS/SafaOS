use core::{mem::ManuallyDrop, ops::Deref};

use safa_abi::raw::io::DirEntry;

use crate::{
    drivers::vfs::CollectionIterDescriptor,
    scheduler::resources::{self, Resource},
};

/// a wrapper around a DirIterDescriptor resource which closes the diriter when dropped
pub struct DirIter(pub(super) usize);

impl DirIter {
    /// Creates a new `DirIter` from a resource index.
    /// takes ownership of the resource index, meaning that the resource will be closed when the `DirIter` is dropped.
    pub fn from_ri(ri: usize) -> Option<Self> {
        resources::get_resource(ri, |resource| {
            if let Resource::DirIter(_) = *resource {
                Some(Self(ri))
            } else {
                None
            }
        })
        .flatten()
    }

    fn with_diriter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut CollectionIterDescriptor) -> R,
    {
        unsafe {
            resources::get_resource(self.0, |mut resource| {
                let Resource::DirIter(ref mut diriter) = *resource else {
                    unreachable!()
                };

                f(diriter)
            })
            .unwrap_unchecked()
        }
    }

    /// Returns the next directory entry in the directory.
    pub fn next(&self) -> Option<DirEntry> {
        self.with_diriter(|diriter| diriter.next())
    }
}

impl Drop for DirIter {
    fn drop(&mut self) {
        resources::remove_resource(self.0).unwrap();
    }
}

/// a wrapper around [`ManuallyDrop<DirIter>`] which doesn't close the diriter when dropped
pub struct DirIterRef(pub(super) ManuallyDrop<DirIter>);

impl DirIterRef {
    /// Creates a new `DirIterRef` from a resource index.
    /// unlike [`DirIter`], this doesn't close the diriter when dropped and therefore doesn't take ownership of the resource.
    pub fn get(ri: usize) -> Option<Self> {
        let diriter = DirIter::from_ri(ri)?;
        Some(Self(ManuallyDrop::new(diriter)))
    }

    pub fn ri(&self) -> usize {
        self.0.0
    }
}

impl Deref for DirIterRef {
    type Target = DirIter;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
