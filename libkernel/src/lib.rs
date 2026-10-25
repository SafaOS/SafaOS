#![no_std]
#![feature(allocator_api)]
#![feature(const_trait_impl)]
#![feature(const_ops)]
#![feature(try_trait_v2)]
#![feature(sync_unsafe_cell)]

extern crate alloc;
pub mod bootloader;
pub mod collections;
pub mod ffi;
pub mod future;
pub mod kalloc;
pub mod logging;
pub mod misc;
pub mod paging;
pub mod percpu;
pub mod pmm;
pub mod rt;
pub mod slab;
pub mod sync;
pub mod task;
pub mod time;
pub mod vmm;
