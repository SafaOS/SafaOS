mod bus;
mod pci;
mod platform;

pub use bus::*;
pub use pci::*;
pub use platform::*;

use crate::rt::abi::*;

#[repr(C)]
pub struct GenericDevice<'a> {
    pub driver: &'a GenericDriver<'a>,
    pub bus_opaque: OpaqueBusDriver<'a>,
    pub bus_abi_private: *mut (),
    pub drv_private: *mut (),
}

pub struct GenericDriver<'a> {
    name: FFISlice<'a, u8>,
    queries: FFISlice<'a, DeviceBusQuery<'a>>,
    prob: extern "C" fn(&mut GenericDevice) -> bool,
    remove: extern "C" fn(&mut GenericDevice),
}

impl<'a> GenericDriver<'a> {
    pub fn name(&self) -> &'a str {
        str::from_utf8(self.name.into_slice()).expect("Invalid driver name")
    }

    pub fn query(&self) -> &'a [DeviceBusQuery<'a>] {
        self.queries.into_slice()
    }
}

pub trait GenericDriverImpl {
    const DTB_QUERY: &[DeviceTreeQuery<'static>] = &[];
    const ACPI_QUERY: &[ACPIQuery<'static>] = &[];
    const PCI_QUERY: &[PCIDeviceQuery] = &[];
}

pub trait PlatformDriver: Sized {
    const DTB_QUERY: &[DeviceTreeQuery<'static>] = &[];
    const ACPI_QUERY: &[ACPIQuery<'static>] = &[];

    fn query() -> &'static [DeviceBusQuery<'static>] {
        const {
            let dtb = unsafe { DeviceBusQuery::new("platform_v1-devicetree", Self::DTB_QUERY) };
            let acpi = unsafe { DeviceBusQuery::new("platform_v1-acpi", Self::ACPI_QUERY) };

            &[acpi, dtb]
        }
    }

    fn probe(driver: &GenericDriver, bus: OpaqueBusDriver) -> Result<Self, ()>;
    fn remove(driver: &GenericDriver, bus: OpaqueBusDriver);
}

pub trait PCIDriver: Sized {
    const QUERY: &[PCIDeviceQuery] = &[];

    fn query() -> &'static [DeviceBusQuery<'static>] {
        const {
            let pci = unsafe { DeviceBusQuery::new("pci_v1", Self::QUERY) };

            &[pci]
        }
    }

    fn probe(driver: &GenericDriver, bus: OpaqueBusDriver) -> Result<Self, ()>;
    fn remove(driver: &GenericDriver, bus: OpaqueBusDriver);
}
