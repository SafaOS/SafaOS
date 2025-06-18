use core::fmt::Debug;

use alloc::boxed::Box;

use crate::drivers::xhci::{rings::trbs::NormalTRB, usb_endpoint::USBEndpoint, XHCIResponseQueue};

pub trait USBHIDDriver: Debug {
    fn create() -> Self
    where
        Self: Sized;
    fn on_event(&self, data: &[u8]);
}

#[derive(Debug)]
pub struct USBHIDDevice {
    pub endpoint: USBEndpoint,
    inner_driver: Box<dyn USBHIDDriver>,
}

impl USBHIDDevice {
    pub fn create<T: USBHIDDriver + Sized + 'static>(endpoint: USBEndpoint) -> Self {
        Self {
            endpoint,
            inner_driver: Box::new(T::create()),
        }
    }

    pub fn start(&mut self, queue: &XHCIResponseQueue) {
        self.request_hid_report(queue);
    }

    pub fn on_event(&mut self, queue: &XHCIResponseQueue) {
        let data = self.endpoint.data_buffer();
        self.inner_driver.on_event(data);
        self.request_hid_report(queue);
    }

    pub fn request_hid_report(&mut self, queue: &XHCIResponseQueue) {
        let data_base = self.endpoint.data_buffer_base();
        let max_packet_size = self.endpoint.desc().max_packet_size();
        let endpoint_num = self.endpoint.desc().endpoint_num();

        let transfer_ring = self.endpoint.transfer_ring();

        let mut normal_trb = NormalTRB::new(data_base, max_packet_size as u32, 0);
        normal_trb.cmd.set_ioc(true);

        transfer_ring.enqueue(normal_trb.into_trb());
        queue
            .doorbell_manager
            .lock()
            .ring_endpoint_doorbell(transfer_ring.doorbell_id(), endpoint_num);
    }
}
