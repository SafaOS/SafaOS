use crate::rt::CSlice;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ACPIQuery<'a> {
    hid: CSlice<'a, u8>,
}

impl<'a> ACPIQuery<'a> {
    #[inline(always)]
    pub const fn new(hardware_id: &'a str) -> Self {
        Self {
            hid: CSlice::from_slice(hardware_id.as_bytes()),
        }
    }

    #[inline(always)]
    pub fn hardware_id(&self) -> &'a str {
        str::from_utf8(self.hid.into_slice()).expect("ACPI Query corrupted")
    }
}

#[derive(Clone, Copy)]
pub struct DeviceTreeQuery<'a> {
    compatible: CSlice<'a, u8>,
}

impl<'a> DeviceTreeQuery<'a> {
    pub const fn new(compatible: &'a str) -> Self {
        Self {
            compatible: CSlice::from_slice(compatible.as_bytes()),
        }
    }

    pub fn compatible(&self) -> &'a str {
        str::from_utf8(self.compatible.into_slice()).expect("DeviceTree ID corrupted")
    }
}
