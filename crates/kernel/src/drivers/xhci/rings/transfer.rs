use crate::{
    drivers::xhci::{self, rings::trbs::TRB, utils::XHCIError},
    memory::frame_allocator::{self, FramePtr},
    PhysAddr, VirtAddr,
};

/// A Transfer Ring exists for each active endpoint or Stream declared by a USB
/// device. Transfer Rings contain “Transfer” specific TRBs. Section 4.11.2 for more
/// information on Transfer TRBs.
#[derive(Debug)]
pub struct XHCITransferRing {
    trbs_ptr: FramePtr<[TRB]>,
    trbs_len: usize,

    curr_ring_cycle_bit: u8,

    enqueue_ptr: usize,

    doorbell_id: u8,
}

impl XHCITransferRing {
    pub const fn doorbell_id(&self) -> u8 {
        self.doorbell_id
    }

    pub const fn curr_ring_cycle_bit(&self) -> u8 {
        self.curr_ring_cycle_bit
    }

    pub fn create(max_trb_count: usize, doorbell_id: u8) -> Result<Self, XHCIError> {
        let curr_ring_cycle_bit = 1;

        let (trbs, trbs_phys_addr) =
            xhci::utils::allocate_buffers(max_trb_count).ok_or(XHCIError::OutOfMemory)?;
        trbs[max_trb_count - 1] = TRB::new_link(trbs_phys_addr, curr_ring_cycle_bit);

        let trbs_len = trbs.len();
        let trbs_ptr = unsafe { FramePtr::from_ptr(trbs) };

        Ok(Self {
            trbs_ptr,
            trbs_len,
            enqueue_ptr: 0,
            curr_ring_cycle_bit,
            doorbell_id,
        })
    }
    unsafe fn get_trb(&self, index: usize) -> *mut TRB {
        assert!(index < self.trbs_len);
        unsafe { (self.trbs_ptr.as_ptr() as *mut TRB).add(index) }
    }

    unsafe fn write_trb(&mut self, index: usize, trb: TRB) {
        unsafe {
            self.get_trb(index).write_volatile(trb);
        }
    }

    pub fn get_physical_dequeue_pointer_base(&self) -> PhysAddr {
        unsafe { VirtAddr::from_ptr(self.get_trb(self.enqueue_ptr)).into_phys() }
    }

    /// Enqueues a TRB into the current transfer ring
    pub fn enqueue(&mut self, mut trb: TRB) {
        trb.cmd.set_cycle_bit(self.curr_ring_cycle_bit);

        unsafe {
            self.write_trb(self.enqueue_ptr, trb);
        }
        self.enqueue_ptr += 1;

        if self.enqueue_ptr >= self.trbs_len - 1 {
            // Update the link trb to refelect the current cycle
            let link_trb = unsafe { &mut *self.get_trb(self.trbs_len - 1) };
            link_trb.cmd.set_cycle_bit(self.curr_ring_cycle_bit);

            // Start a new cycle
            self.enqueue_ptr = 0;
            self.curr_ring_cycle_bit = (!self.curr_ring_cycle_bit) & 0x1;
        }
    }
}

impl Drop for XHCITransferRing {
    fn drop(&mut self) {
        frame_allocator::deallocate_frame(self.trbs_ptr.frame());
    }
}
