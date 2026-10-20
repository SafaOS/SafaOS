pub struct PCIDeviceQuery {
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub vendor_id: u16,
    pub device_id: u16,
}

impl PCIDeviceQuery {
    /// Constructs a new PCI device lookup query that matches any `class`-`subclass` pair
    pub const fn new(class: u8, subclass: u8) -> Self {
        Self {
            class,
            subclass,
            prog_if: u8::MAX,
            vendor_id: u16::MAX,
            device_id: u16::MAX,
        }
    }

    /// Resterics the lookup by a vendor id.
    pub const fn with_vendor_id(mut self, vendor_id: u16) -> Self {
        self.vendor_id = vendor_id;
        self
    }

    /// Resterics the lookup by a device id.
    pub const fn with_device_id(mut self, device_id: u16) -> Self {
        self.device_id = device_id;
        self
    }
}
