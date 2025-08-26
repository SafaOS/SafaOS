use serde::Serialize;

use crate::{
    PhysAddr,
    drivers::xhci::{
        MAX_TRB_COUNT, rings::transfer::XHCITransferRing, usb::UsbEndpointDescriptor,
        utils::XHCIError,
    },
    memory::{
        frame_allocator::{self, FramePtr, RegionListAllocator},
        paging::PAGE_SIZE,
    },
};

#[derive(Debug)]
pub struct USBEndpoint {
    descriptor: UsbEndpointDescriptor,
    transfer_ring: XHCITransferRing,
    data_buffer: FramePtr<[u8; PAGE_SIZE]>,
}

impl Serialize for USBEndpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.descriptor.serialize(serializer)
    }
}

impl USBEndpoint {
    pub fn create(
        allocator: &mut RegionListAllocator,
        descriptor: UsbEndpointDescriptor,
        slot_id: u8,
    ) -> Result<Self, XHCIError> {
        let data_frame = allocator.allocate_frame().ok_or(XHCIError::OutOfMemory)?;
        Ok(Self {
            descriptor,
            transfer_ring: XHCITransferRing::create(allocator, MAX_TRB_COUNT, slot_id)?,
            data_buffer: unsafe { data_frame.into_ptr() },
        })
    }

    pub fn transfer_ring(&mut self) -> &mut XHCITransferRing {
        &mut self.transfer_ring
    }

    pub fn data_buffer_base(&self) -> PhysAddr {
        self.data_buffer.phys_addr()
    }

    pub fn data_buffer(&self) -> &[u8; PAGE_SIZE] {
        &*self.data_buffer
    }

    pub fn desc(&self) -> &UsbEndpointDescriptor {
        &self.descriptor
    }
}

impl Drop for USBEndpoint {
    fn drop(&mut self) {
        frame_allocator::allocator().deallocate_frame(self.data_buffer.frame());
    }
}
