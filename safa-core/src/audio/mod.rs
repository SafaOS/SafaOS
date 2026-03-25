use alloc::boxed::Box;

use crate::{
    audio::interface::{AudioCard, AudioDev},
    drivers::vfs::VFS_STRUCT,
};

pub mod interface;

pub fn register_interface(interface: &'static dyn AudioCard) {
    crate::devices::add_device_at(&*VFS_STRUCT.read(), Box::new(AudioDev(interface)), "audio");
}
