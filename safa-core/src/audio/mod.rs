use alloc::boxed::Box;

use crate::{
    audio::interface::{AudioCard, AudioDev, BoxAudioDev},
    drivers::vfs::VFS_STRUCT,
};

pub mod interface;

pub fn register_interface(interface: &'static dyn AudioCard) {
    crate::devices::add_device_at(&*VFS_STRUCT.read(), Box::new(AudioDev(interface)), "audio");
}

pub fn register_stream(name: &str, stream: Box<dyn AudioCard>) {
    // FIXME: 3 pointers of indirection...
    crate::devices::add_device_at_with_name(
        &*VFS_STRUCT.read(),
        Box::new(BoxAudioDev(stream)),
        "audio",
        Some(name),
    );
}
