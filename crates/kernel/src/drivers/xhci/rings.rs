#![allow(dead_code)]
use modular_bitfield::prelude::*;

use crate::PhysAddr;

use super::utils::allocate_buffers;

#[bitfield]
#[derive(Debug, Clone, Copy)]
pub struct TRBCommand {
    cycle_bit: B1,
    toggle_cycle: B1,
    #[skip]
    reserved0: B8,
    trb_type: B6,
    #[skip]
    reserved1: B16,
}

impl TRBCommand {
    fn as_u32(&self) -> u32 {
        unsafe { core::mem::transmute(*self) }
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct TRB {
    parameter: u64,
    status: u32,
    cmd: TRBCommand,
}

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
        link_trb.cmd.set_trb_type(6);
        link_trb.cmd.set_toggle_cycle(1);
        link_trb.cmd.set_cycle_bit(1);

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

// size is hard to tell with `TRBCommand`
const _: () = assert!(size_of::<TRB>() == size_of::<u64>() * 2);
