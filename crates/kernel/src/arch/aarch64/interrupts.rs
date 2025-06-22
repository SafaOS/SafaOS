use alloc::vec::Vec;

use crate::arch::aarch64::gic::its::commands::{ITSCommand, GITS_COMMAND_QUEUE};
use crate::arch::aarch64::gic::{its, LPI_MANAGER};
use crate::drivers::interrupts::IRQInfo;
use crate::utils::locks::Mutex;
use lazy_static::lazy_static;

pub const IRQS: [u32; 7] = [0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006];
lazy_static! {
    /// a list of device IDs that are already mapped
    static ref MAPPED_DEVICE_IDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
}
pub unsafe fn register_irq_handler(int_id: u32, info: &IRQInfo) {
    let device_id = match info {
        IRQInfo::MSIX(msix) => msix.requester_id(),
    };
    let event_id = int_id;

    let mut mapped_device_ids = MAPPED_DEVICE_IDS.lock();

    // building commands
    let mut command_queue = GITS_COMMAND_QUEUE.lock();

    // map deviceID if it isn't already mapped
    if !mapped_device_ids.contains(&device_id) {
        let (itt_addr, _, itt_range) = its::allocate_itt();
        command_queue.add_command(ITSCommand::new_mapd(
            device_id as u32,
            itt_range,
            itt_addr.into_phys(),
            true,
        ));
        mapped_device_ids.push(device_id);
    }
    drop(mapped_device_ids);

    // enable LPI
    LPI_MANAGER.lock().enable(event_id);
    // map the LPI to the deviceID and collection 0 (should be mapped to processor 0 )
    command_queue.add_command(ITSCommand::new_mapi(device_id as u32, event_id, 0));
    // invalidate to make sure LPI is enabled
    command_queue.add_command(ITSCommand::new_inv(device_id as u32, event_id));
    // Syncs commands for Processor 0 to make sure all of the previous changes are applied
    command_queue.add_command(ITSCommand::sync());
    // waits for all commands to complete
    command_queue.poll();
}
