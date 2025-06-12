use crate::{debug, drivers::xhci::trb::TRB, write_ref, PhysAddr};

use super::{
    regs::{EventRingDequePtr, InterrupterRegs},
    utils::allocate_buffers,
};

use alloc::vec::Vec;

#[derive(Debug)]
// TODO: move to another file
pub struct XHCICommandRing<'s> {
    enqueue_ptr: usize,
    // TODO: free this on drop?
    trbs_phys_addr: PhysAddr,
    trbs: &'s mut [TRB],
    curr_ring_cycle_bit: u8,
}

impl<'s> XHCICommandRing<'s> {
    pub fn create(trb_count: usize) -> Self {
        let (trbs, trbs_phys_addr) = allocate_buffers::<TRB>(trb_count)
            .expect("failed to allocated the XHCI Command Ring TRBs buffer");
        let link_trb = &mut trbs[trb_count - 1];

        link_trb.parameter = trbs_phys_addr.into_raw() as u64;
        link_trb.cmd.set_trb_type(super::trb::TRB_TYPE_LINK);
        link_trb.cmd.set_toggle_cycle(true);
        link_trb.cmd.set_cycle_bit(1);

        debug!(
            XHCICommandRing,
            "created with {} TRBS at {:?}", trb_count, trbs_phys_addr
        );
        Self {
            trbs_phys_addr,
            trbs,
            enqueue_ptr: 0,
            curr_ring_cycle_bit: 1,
        }
    }

    pub fn enqueue(&mut self, mut trb: TRB) {
        trb.cmd.set_cycle_bit(self.curr_ring_cycle_bit);

        self.trbs[self.enqueue_ptr] = trb;
        self.enqueue_ptr += 1;

        if self.enqueue_ptr >= self.trbs.len() - 1 {
            // Update the link trb to refelect the current cycle
            let link_trb = &mut self.trbs[self.trbs.len() - 1];
            link_trb.cmd.set_cycle_bit(self.curr_ring_cycle_bit);

            // Start a new cycle
            self.enqueue_ptr = 0;
            self.curr_ring_cycle_bit = !self.curr_ring_cycle_bit;
        }
    }

    pub fn base_phys_addr(&self) -> PhysAddr {
        self.trbs_phys_addr
    }

    pub fn current_ring_cycle(&self) -> u8 {
        self.curr_ring_cycle_bit
    }
}

/**
> xHci Spec Section 6.5 Event Ring Segment Table Figure 6-40: Event Ring Segment Table Entry

Note: The Ring Segment Size may be set to any value from 16 to 4096, however
software shall allocate a buffer for the Event Ring Segment that rounds up its
size to the nearest 64B boundary to allow full cache-line accesses.
*/
#[repr(C)]
#[derive(Clone, Debug)]
struct XHCIEventRingEntry {
    ring_segment_base: PhysAddr,
    /// Size of the Event Ring Segment (only the lower 16bits are used)
    ring_segment_size: u32,
    __: u32,
}

#[derive(Debug)]
pub struct XHCIEventRing<'a> {
    interrupter_registers: &'a mut InterrupterRegs,

    trbs: &'a mut [TRB],
    trbs_phys_base: PhysAddr,

    ring_segment_table: &'a mut [XHCIEventRingEntry],
    segment_table_base: PhysAddr,

    dequeue_ptr: usize,
    curr_ring_cycle_bit: u8,
}

impl<'a> XHCIEventRing<'a> {
    pub fn create(trb_count: usize, interrupter_registers: &'a mut InterrupterRegs) -> Self {
        let (trbs, trbs_phys_base) = allocate_buffers::<TRB>(trb_count)
            .expect("failed to allocate the XHCI Event Ring TRBs buffer");

        let segment_count = 1;
        let (segment_table, segment_table_base_addr) =
            allocate_buffers::<XHCIEventRingEntry>(segment_count)
                .expect("failed too allocate the XHCI Event Ring Segment table");

        segment_table[0].ring_segment_base = trbs_phys_base;
        segment_table[0].ring_segment_size = trb_count as u32;
        segment_table[0].__ = 0;

        let mut this = Self {
            trbs_phys_base,
            trbs,
            interrupter_registers,
            segment_table_base: segment_table_base_addr,
            ring_segment_table: segment_table,
            dequeue_ptr: 0,
            curr_ring_cycle_bit: 1,
        };
        this.reset();

        debug!(
            XHCIEventRing,
            "created with {} TRBS at {:?}",
            this.trbs.len(),
            this.trbs_phys_base
        );
        this
    }
    pub fn reset(&mut self) {
        // Initializes the interrupter must be done in the order given here:
        write_ref!(
            self.interrupter_registers.erst_sz,
            self.ring_segment_table.len() as u32
        );
        self.update_edrp();
        write_ref!(
            self.interrupter_registers.erst_base,
            self.segment_table_base
        );
    }

    /// Update edrp in the interrupter to sync with the current dequeue pointer
    pub fn update_edrp(&mut self) {
        let offset = self.dequeue_ptr * size_of::<TRB>();
        let dequeue_addr = self.trbs_phys_base + offset;
        write_ref!(
            self.interrupter_registers.event_ring_deque,
            EventRingDequePtr::from_addr(dequeue_addr)
        );
    }

    pub fn dequeue_events(&mut self) -> Vec<TRB> {
        let mut results = Vec::new();
        while let Some(next) = self.dequeue_trb() {
            results.push(next.clone());
        }

        self.update_edrp();
        let edrp = self
            .interrupter_registers
            .event_ring_deque
            .with_handler_busy(true);
        write_ref!(self.interrupter_registers.event_ring_deque, edrp);
        results
    }

    fn dequeue_trb(&mut self) -> Option<&TRB> {
        let curr_trb = &self.trbs[self.dequeue_ptr];
        if curr_trb.cmd.cycle_bit() != self.curr_ring_cycle_bit {
            return None;
        }

        self.dequeue_ptr += 1;
        if self.dequeue_ptr >= self.trbs.len() {
            self.dequeue_ptr = 0;
            self.curr_ring_cycle_bit = !self.curr_ring_cycle_bit;
        }

        Some(curr_trb)
    }
}
