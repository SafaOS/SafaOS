use bitfield_struct::bitfield;

use crate::drivers::xhci::devices::DeviceEndpointType;

pub const USB_DESCRIPTOR_DEVICE_TYPE: u16 = 1;
pub const USB_DESCRIPTOR_CONFIGURATION_TYPE: u16 = 2;
pub const USB_DESCRIPTOR_INTERFACE_TYPE: u16 = 0x04;
pub const USB_DESCRIPTOR_ENDPOINT_TYPE: u16 = 0x05;
pub const USB_DESCRIPTOR_HID_TYPE: u16 = 0x21;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct UsbDescriptorHeader {
    pub b_length: u8,
    pub b_descriptor_type: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct UsbDeviceDescriptor {
    pub header: UsbDescriptorHeader,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_subclass: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size_0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub num_configurations: u8,
}

const _: () = assert!(size_of::<UsbDeviceDescriptor>() == 18);

#[derive(Debug)]
#[repr(C)]
pub struct UsbConfigurationDescriptor {
    pub header: UsbDescriptorHeader,
    pub w_total_len: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration_value: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
    pub data: [u8; 245],
}

const _: () = assert!(size_of::<UsbConfigurationDescriptor>() == 254);

impl UsbConfigurationDescriptor {
    pub fn into_iterator(self) -> UsbInterfaceDescriptorsIter {
        UsbInterfaceDescriptorsIter(UsbInterfaceDescriptorsIterRaw {
            index: 0,
            inner: self,
        })
    }
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct UsbInterfaceDescriptor {
    pub header: UsbDescriptorHeader,
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_subclass: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}

const _: () = assert!(size_of::<UsbInterfaceDescriptor>() == 9);

#[bitfield(u8)]
pub struct EndpointAddr {
    #[bits(4)]
    pub endpoint_num_base: u8,
    #[bits(3)]
    __: (),
    pub direction_in: bool,
}

#[bitfield(u8)]
pub struct EndpointAttrs {
    #[bits(2)]
    pub transfer_type: u8,
    #[bits(2)]
    pub sync_type: u8,
    #[bits(2)]
    pub usage_type: u8,
    #[bits(2)]
    __: (),
}

#[derive(Debug, Clone)]
#[repr(C, packed)]
pub struct UsbEndpointDescriptor {
    pub header: UsbDescriptorHeader,
    pub b_endpoint_addr: EndpointAddr,
    pub bm_attributes: EndpointAttrs,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}
const _: () = assert!(size_of::<UsbEndpointDescriptor>() == 7);

impl UsbEndpointDescriptor {
    pub const fn endpoint_num(&self) -> u8 {
        (self.b_endpoint_addr.endpoint_num_base() * 2) + self.b_endpoint_addr.direction_in() as u8
    }

    pub const fn max_packet_size(&self) -> u16 {
        self.w_max_packet_size & 0x7FF
    }

    pub const fn endpoint_type(&self) -> DeviceEndpointType {
        match self.bm_attributes.transfer_type() {
            0b00 => DeviceEndpointType::ControlBI,
            0b01 => {
                if self.b_endpoint_addr.direction_in() {
                    DeviceEndpointType::IsochIn
                } else {
                    DeviceEndpointType::IsochOut
                }
            }
            0b10 => {
                if self.b_endpoint_addr.direction_in() {
                    DeviceEndpointType::BulkIn
                } else {
                    DeviceEndpointType::BulkOut
                }
            }
            0b11 => {
                if self.b_endpoint_addr.direction_in() {
                    DeviceEndpointType::IntIn
                } else {
                    DeviceEndpointType::IntOut
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct UsbHIDDescriptorDesc {
    pub b_descriptor_type: u8,
    pub w_descriptor_length: u16,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct UsbHIDDescriptor {
    pub header: UsbDescriptorHeader,
    pub bcd_hid: u16,
    pub b_country_code: u8,
    pub b_num_descriptors: u8,
    pub desc: [UsbHIDDescriptorDesc; 1],
}

const _: () = assert!(size_of::<UsbHIDDescriptor>() == 9);

#[derive(Debug)]
pub enum GenericUSBDescriptor {
    Interface(UsbInterfaceDescriptor),
    Endpoint(UsbEndpointDescriptor),
    #[allow(unused)]
    HID(UsbHIDDescriptor),
}

/// A raw iterator over Usb Interface Descriptors
struct UsbInterfaceDescriptorsIterRaw {
    index: usize,
    inner: UsbConfigurationDescriptor,
}

impl Iterator for UsbInterfaceDescriptorsIterRaw {
    type Item = *const UsbDescriptorHeader;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= (self.inner.w_total_len as usize - self.inner.header.b_length as usize) {
            return None;
        }

        let header: Self::Item = (&raw const self.inner.data[self.index]).cast();

        unsafe {
            self.index += (*header).b_length as usize;
            Some(header)
        }
    }
}

/// A safe iterator over Usb Interface Descriptors
pub struct UsbInterfaceDescriptorsIter(UsbInterfaceDescriptorsIterRaw);
impl Iterator for UsbInterfaceDescriptorsIter {
    type Item = GenericUSBDescriptor;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().and_then(|x| unsafe {
            let header = x.read_unaligned();
            Some(match header.b_descriptor_type as u16 {
                USB_DESCRIPTOR_INTERFACE_TYPE => GenericUSBDescriptor::Interface(
                    x.cast::<UsbInterfaceDescriptor>().read_unaligned(),
                ),
                USB_DESCRIPTOR_ENDPOINT_TYPE => GenericUSBDescriptor::Endpoint(
                    x.cast::<UsbEndpointDescriptor>().read_unaligned(),
                ),
                USB_DESCRIPTOR_HID_TYPE => {
                    GenericUSBDescriptor::HID(x.cast::<UsbHIDDescriptor>().read_unaligned())
                }
                _ => return None,
            })
        })
    }
}
