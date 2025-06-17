#[derive(Debug)]
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

pub const USB_DESCRIPTOR_CONFIGURATION_TYPE: u16 = 2;
pub const USB_DESCRIPTOR_DEVICE_TYPE: u16 = 1;
