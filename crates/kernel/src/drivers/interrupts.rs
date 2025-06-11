use core::fmt::Debug;

use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::arch::interrupts::IRQS;

use super::pci::msi::MSIXInfo;
use crate::utils::locks::Mutex;

pub trait InterruptReceiver: Send + Sync + Debug {
    fn handle_interrupt(&self);
}

#[derive(Debug, Clone, Copy)]
pub enum IntTrigger {
    Edge,
    #[allow(unused)]
    LevelDeassert,
    #[allow(unused)]
    LevelAssert,
}

#[derive(Debug, Clone)]
pub enum IRQInfo {
    MSIX(MSIXInfo),
}

unsafe impl Send for IRQInfo {}
unsafe impl Sync for IRQInfo {}

impl IRQInfo {
    fn setup(&mut self, irq_num: u32, trigger: IntTrigger) {
        match self {
            IRQInfo::MSIX(ref mut msix) => msix.setup(irq_num, trigger),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IRQ {
    info: IRQInfo,
    trigger: IntTrigger,
    pub handler: &'static dyn InterruptReceiver,
    pub irq_num: u32,
}

impl IRQ {
    fn setup(&mut self, irq_num: u32) {
        self.info.setup(irq_num, self.trigger);
    }

    pub const fn new(
        info: IRQInfo,
        trigger: IntTrigger,
        handler: &'static dyn InterruptReceiver,
        irq_num: u32,
    ) -> Self {
        Self {
            info,
            trigger,
            handler,
            irq_num,
        }
    }
}

/// An abstraction layer over the architecture's IRQ mangament
pub struct IRQManager {
    free_irq_nums: heapless::Vec<u32, { IRQS.len() }>,
    next_irq_num_index: usize,
    pub irqs: Vec<IRQ>,
}

impl IRQManager {
    pub fn register_irq(
        &mut self,
        irq_info: IRQInfo,
        triggering: IntTrigger,
        handler: &'static dyn InterruptReceiver,
    ) {
        let irq_num = self.free_irq_nums[self.next_irq_num_index];
        unsafe {
            crate::arch::interrupts::register_irq_handler(irq_num, &irq_info);
            let mut irq = IRQ::new(irq_info, triggering, handler, irq_num);
            irq.setup(irq_num);

            self.irqs.push(irq);
        }
        self.next_irq_num_index += 1;
    }

    pub fn new() -> Self {
        let mut free_irq_nums = heapless::Vec::new();
        for irq_num in IRQS {
            free_irq_nums.push(irq_num).unwrap();
        }

        Self {
            free_irq_nums,
            next_irq_num_index: 0,
            irqs: Vec::new(),
        }
    }
}

lazy_static! {
    pub static ref IRQ_MANAGER: Mutex<IRQManager> = Mutex::new(IRQManager::new());
}

/// Register an IRQ handler according to `irq_info` (eg. MSIX or MSI)
pub fn register_irq(
    irq_info: IRQInfo,
    triggering: IntTrigger,
    handler: &'static dyn InterruptReceiver,
) {
    IRQ_MANAGER
        .lock()
        .register_irq(irq_info, triggering, handler);
}
