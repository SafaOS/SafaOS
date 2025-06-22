use std::{
    fs::File,
    io::{self, BufReader},
};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct USBEndpoint {}

#[derive(Deserialize)]
pub struct USBInterfaceDescriptor {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
}

impl USBInterfaceDescriptor {
    pub const fn class(&self) -> u8 {
        self.b_interface_class
    }

    pub const fn subclass(&self) -> u8 {
        self.b_interface_subclass
    }

    pub const fn protocol(&self) -> u8 {
        self.b_interface_protocol
    }
}

#[derive(Deserialize)]
pub struct USBInterface {
    descriptor: USBInterfaceDescriptor,
    endpoints: Vec<USBEndpoint>,
    has_driver: bool,
}

impl USBInterface {
    pub const fn descriptor(&self) -> &USBInterfaceDescriptor {
        &self.descriptor
    }

    pub const fn has_driver(&self) -> bool {
        self.has_driver
    }

    pub fn endpoints(&self) -> &[USBEndpoint] {
        &self.endpoints
    }
}

#[derive(Deserialize)]
pub struct USBDeviceDescriptor {
    id_vendor: u16,
    id_product: u16,
    b_device_class: u8,
    b_device_subclass: u8,
    b_device_protocol: u8,
    #[serde(rename = "b_max_packet_size_0")]
    b_max_packet_size: u8,
}

impl USBDeviceDescriptor {
    pub const fn id_vendor(&self) -> u16 {
        self.id_vendor
    }

    pub const fn id_product(&self) -> u16 {
        self.id_product
    }

    pub const fn class(&self) -> u8 {
        self.b_device_class
    }

    pub const fn subclass(&self) -> u8 {
        self.b_device_subclass
    }

    pub const fn protocol(&self) -> u8 {
        self.b_device_protocol
    }

    pub const fn max_packet_size(&self) -> u8 {
        self.b_max_packet_size
    }
}

#[derive(Deserialize)]
pub struct USBDevice {
    manufacturer: String,
    product: String,
    serial_number: String,
    descriptor: USBDeviceDescriptor,
    slot_id: u8,
    interfaces: Vec<USBInterface>,
}

impl USBDevice {
    pub const fn descriptor(&self) -> &USBDeviceDescriptor {
        &self.descriptor
    }

    pub fn manufacturer(&self) -> &str {
        &self.manufacturer
    }

    pub fn product(&self) -> &str {
        &self.product
    }

    pub fn serial_number(&self) -> &str {
        &self.serial_number
    }

    pub const fn slot_id(&self) -> u8 {
        self.slot_id
    }

    pub const fn interfaces(&self) -> &Vec<USBInterface> {
        &self.interfaces
    }
}

#[derive(Deserialize)]
pub struct USBInfo {
    connected_devices: Vec<USBDevice>,
}

impl USBInfo {
    pub fn fetch() -> io::Result<Self> {
        let file = File::open("proc:/usbinfo")?;
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }

    pub const fn connected_devices(&self) -> &Vec<USBDevice> {
        &self.connected_devices
    }
}
