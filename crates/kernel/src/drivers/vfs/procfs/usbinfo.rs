use alloc::vec::Vec;
use serde::{ser::SerializeStruct, Serialize};

use crate::{
    drivers::{pci::XHCI_DEVICE, vfs::procfs::ProcFSFile, xhci::usb_device::USBDevice},
    utils::{alloc::PageString, locks::RwLockReadGuard},
};

pub struct USBInfo<'a> {
    connected_devices: RwLockReadGuard<'a, Vec<USBDevice>>,
}

impl Serialize for USBInfo<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut structure = serializer.serialize_struct("USBInfo", 1)?;
        structure.serialize_field("connected_devices", &**self.connected_devices)?;
        structure.end()
    }
}

impl<'a> USBInfo<'a> {
    pub fn fetch() -> Option<Self> {
        let connected_devices = XHCI_DEVICE.as_ref()?.read_connected_devices();
        Some(Self { connected_devices })
    }
}

pub struct USBInfoFile;

impl USBInfoFile {
    pub fn new() -> ProcFSFile {
        ProcFSFile::new("usbinfo", 0, Self::fetch)
    }

    pub fn fetch(_: &mut ProcFSFile) -> Option<PageString> {
        let mut page_string = PageString::with_capacity(1024);
        let mem_info = USBInfo::fetch();

        serde_json::to_writer_pretty(&mut page_string, &mem_info)
            .ok()
            .map(|()| page_string)
    }
}
