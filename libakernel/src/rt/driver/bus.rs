use macros::vtable;

use crate::ffi::*;
use crate::rt::driver::{GenericDevice, GenericDriver};

#[repr(C)]
pub struct OpaqueBusDriver<'a> {
    vtable: &'a GenericBusDriverVTable,
    this: OpaqueSelf<'a>,
}

#[repr(C)]
#[derive(Clone, Copy)]
/// Generic list of queries for a specific bus.
pub struct DeviceBusQuery<'a> {
    bus_name: FFISlice<'a, u8>,
    ptr: *const (),
    len: usize,
}

impl<'a> DeviceBusQuery<'a> {
    #[inline(always)]
    /// Safety: layout of queries must be valid for `bus_name`
    pub const unsafe fn new<T>(bus_name: &'a str, queries: &'a [T]) -> Self {
        Self {
            bus_name: FFISlice::from_slice(bus_name.as_bytes()),
            ptr: queries.as_ptr().cast(),
            len: queries.len(),
        }
    }
}

#[vtable]
pub trait GenericBusDriver {
    fn match_drv(&self, drv: &GenericDriver) -> bool;
    // After match is called
    fn prob_drv(&self, device: &mut GenericDevice);
    fn abi_name(&self) -> &str;
}
