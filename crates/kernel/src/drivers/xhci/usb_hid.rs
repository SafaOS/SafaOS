use core::fmt::Debug;

use crate::drivers::xhci::{
    rings::trbs::NormalTRB, usb_endpoint::USBEndpoint, usb_interface::USBInterfaceDriver,
    XHCIResponseQueue,
};

pub fn request_hid_report(endpoint: &mut USBEndpoint, queue: &XHCIResponseQueue) {
    let data_base = endpoint.data_buffer_base();
    let max_packet_size = endpoint.desc().max_packet_size();
    let endpoint_num = endpoint.desc().endpoint_num();

    let transfer_ring = endpoint.transfer_ring();

    let mut normal_trb = NormalTRB::new(data_base, max_packet_size as u32, 0);
    normal_trb.cmd.set_ioc(true);

    transfer_ring.enqueue(normal_trb.into_trb());
    queue
        .doorbell_manager
        .lock()
        .ring_endpoint_doorbell(transfer_ring.doorbell_id(), endpoint_num);
}

pub trait USBHIDDriver: Debug {
    fn create() -> Self
    where
        Self: Sized;
    fn on_event(&mut self, data: &[u8]);
}

impl<T: USBHIDDriver + Sized + 'static> USBInterfaceDriver for T {
    fn create() -> Self
    where
        Self: Sized,
    {
        USBHIDDriver::create()
    }

    fn start(&mut self, endpoints: &mut [USBEndpoint], queue: &XHCIResponseQueue) {
        let endpoint = &mut endpoints[0];
        request_hid_report(endpoint, queue);
    }

    fn on_event(&mut self, endpoint: &mut [USBEndpoint], queue: &XHCIResponseQueue) {
        let endpoint = &mut endpoint[0];
        let data = endpoint.data_buffer();
        self.on_event(data);
        request_hid_report(endpoint, queue);
    }
}
