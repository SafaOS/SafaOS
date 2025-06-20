use core::fmt::Debug;

use alloc::{boxed::Box, vec::Vec};

use crate::drivers::xhci::{
    usb::{UsbEndpointDescriptor, UsbInterfaceDescriptor},
    usb_endpoint::USBEndpoint,
    utils::XHCIError,
    XHCIResponseQueue,
};

pub trait USBInterfaceDriver: Debug {
    fn create() -> Self
    where
        Self: Sized;
    fn on_event(&mut self, endpoints: &mut [USBEndpoint], queue: &XHCIResponseQueue);
    fn start(&mut self, endpoints: &mut [USBEndpoint], queue: &XHCIResponseQueue);
}

#[derive(Debug)]
pub struct USBInterface {
    slot_id: u8,
    interface_descriptor: UsbInterfaceDescriptor,

    endpoints: Vec<USBEndpoint>,
    driver: Option<Box<dyn USBInterfaceDriver>>,
}

impl USBInterface {
    pub fn endpoints(&mut self) -> &mut [USBEndpoint] {
        &mut self.endpoints
    }

    pub fn new(
        descriptor: UsbInterfaceDescriptor,
        endpoints_desc: Vec<UsbEndpointDescriptor>,
        slot_id: u8,
    ) -> Result<Self, XHCIError> {
        assert_eq!(descriptor.b_num_endpoints as usize, endpoints_desc.len());

        let mut endpoints = Vec::with_capacity(endpoints_desc.len());
        for endpoint_desc in endpoints_desc {
            endpoints.push(USBEndpoint::create(endpoint_desc, slot_id)?);
        }

        Ok(Self {
            slot_id,
            interface_descriptor: descriptor,
            endpoints,
            driver: None,
        })
    }

    pub const fn desc(&self) -> &UsbInterfaceDescriptor {
        &self.interface_descriptor
    }

    pub const fn slot_id(&self) -> u8 {
        self.slot_id
    }

    pub fn start(&mut self, queue: &XHCIResponseQueue) {
        if let Some(driver) = self.driver.as_mut() {
            driver.start(&mut self.endpoints, queue);
        }
    }

    pub fn on_event(&mut self, queue: &XHCIResponseQueue) {
        if let Some(driver) = self.driver.as_mut() {
            driver.on_event(&mut self.endpoints, queue);
        }
    }

    pub fn attach_driver<T: USBInterfaceDriver + 'static + Sized>(&mut self) {
        self.driver = Some(Box::new(T::create()));
    }
}
