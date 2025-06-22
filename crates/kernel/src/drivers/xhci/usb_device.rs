use alloc::{string::String, vec::Vec};
use serde::{ser::SerializeStruct, Serialize};

use crate::drivers::xhci::{
    usb::UsbDeviceDescriptor, usb_interface::USBInterface, XHCIResponseQueue,
};

#[derive(Debug)]
pub struct USBDevice {
    manufacturer: String,
    product: String,
    serial_number: String,

    descriptor: UsbDeviceDescriptor,
    slot_id: u8,
    interfaces: Vec<USBInterface>,
}

impl Serialize for USBDevice {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("USBDevice", 5)?;

        state.serialize_field("manufacturer", &self.manufacturer)?;
        state.serialize_field("product", &self.product)?;
        state.serialize_field("serial_number", &self.serial_number)?;

        state.serialize_field("descriptor", &self.descriptor)?;
        state.serialize_field("slot_id", &self.slot_id)?;
        state.serialize_field("interfaces", self.interfaces())?;
        state.end()
    }
}

impl USBDevice {
    pub fn new(
        manufacturer: String,
        product: String,
        serial_number: String,
        descriptor: UsbDeviceDescriptor,
        slot_id: u8,
        interfaces: Vec<USBInterface>,
    ) -> Self {
        Self {
            manufacturer,
            product,
            serial_number,
            descriptor,
            slot_id,
            interfaces,
        }
    }

    pub const fn slot_id(&self) -> u8 {
        self.slot_id
    }

    pub fn on_event(&mut self, queue: &XHCIResponseQueue, target_endpoint_num: u8) {
        let interfaces = self.interfaces_mut();
        let interfaces = interfaces.into_iter();
        let interfaces = interfaces.filter(|i| {
            i.endpoints()
                .iter()
                .any(|e| e.desc().endpoint_num() == target_endpoint_num)
        });

        for interface in interfaces {
            interface.on_event(queue);
        }
    }

    pub fn interfaces(&self) -> &[USBInterface] {
        &self.interfaces
    }

    pub fn interfaces_mut(&mut self) -> &mut [USBInterface] {
        &mut self.interfaces
    }
}
