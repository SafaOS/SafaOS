use core::fmt::Debug;

use alloc::{boxed::Box, vec::Vec};
use serde::{ser::SerializeStruct, Serialize};

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
    descriptor: UsbInterfaceDescriptor,
    endpoints: Vec<USBEndpoint>,
    driver: Option<Box<dyn USBInterfaceDriver>>,
}

impl Serialize for USBInterface {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("USBInterface", 1)?;
        state.serialize_field("descriptor", &self.descriptor)?;
        state.serialize_field("endpoints", &self.endpoints)?;
        state.serialize_field("has_driver", &self.driver.is_some())?;
        state.end()
    }
}

impl USBInterface {
    pub fn endpoints_mut(&mut self) -> &mut [USBEndpoint] {
        &mut self.endpoints
    }

    pub fn endpoints(&self) -> &[USBEndpoint] {
        &self.endpoints
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
            descriptor,
            endpoints,
            driver: None,
        })
    }

    pub const fn desc(&self) -> &UsbInterfaceDescriptor {
        &self.descriptor
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
