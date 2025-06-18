use crate::{
    drivers::xhci::{
        rings::transfer::XHCITransferRing, usb::UsbEndpointDescriptor, utils::XHCIError,
        MAX_TRB_COUNT,
    },
    memory::{
        frame_allocator::{self, FramePtr},
        paging::PAGE_SIZE,
    },
    PhysAddr,
};

#[derive(Debug)]
pub struct USBEndpoint {
    desc: UsbEndpointDescriptor,
    transfer_ring: XHCITransferRing,
    data_buffer: FramePtr<[u8; PAGE_SIZE]>,
}

impl USBEndpoint {
    pub fn create(descriptor: UsbEndpointDescriptor, slot_id: u8) -> Result<Self, XHCIError> {
        let data_frame = frame_allocator::allocate_frame().ok_or(XHCIError::OutOfMemory)?;
        Ok(Self {
            desc: descriptor,
            transfer_ring: XHCITransferRing::create(MAX_TRB_COUNT, slot_id)?,
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
        &self.desc
    }
}

impl Drop for USBEndpoint {
    fn drop(&mut self) {
        frame_allocator::deallocate_frame(self.data_buffer.frame());
    }
}
