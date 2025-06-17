#[derive(Debug)]
#[repr(C)]
pub struct UsbDescriptorHeader {
    b_length: u8,
    b_descriptor_type: u8,
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
