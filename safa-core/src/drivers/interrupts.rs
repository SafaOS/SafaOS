use core::fmt::Debug;

use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::{drivers::pci::msi::MSIInfo, utils::locks::RwLock};

use super::pci::msi::MSIXInfo;

pub trait InterruptReceiver: Send + Sync + Debug {
    fn handle_interrupt(&'static self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    MSI(MSIInfo),
    PCIInt {
        bus: u8,
        device: u8,
        function: u8,
        interrupt_line: u8,
        interrupt_pin: u8,
    },
}

unsafe impl Send for IRQInfo {}
unsafe impl Sync for IRQInfo {}

impl IRQInfo {
    fn setup(&mut self, irq_num: u32, trigger: IntTrigger) {
        match self {
            IRQInfo::MSIX(msix) => msix.setup(irq_num, trigger),
            IRQInfo::MSI(msi) => msi.setup(irq_num, trigger),
            IRQInfo::PCIInt { .. } => {}
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

/// An abstraction layer over the architecture's IRQ management
pub struct IRQManager {
    pub irqs: Vec<IRQ>,
}

impl IRQManager {
    pub fn register_irq(
        &mut self,
        irq_info: IRQInfo,
        triggering: IntTrigger,
        handler: &'static dyn InterruptReceiver,
    ) {
        unsafe {
            let irq_num = crate::arch::interrupts::register_irq_handler(&irq_info, triggering);
            let mut irq = IRQ::new(irq_info, triggering, handler, irq_num);
            irq.setup(irq_num);

            self.irqs.push(irq);
        }
    }

    pub fn new() -> Self {
        Self { irqs: Vec::new() }
    }
}

lazy_static! {
    pub static ref IRQ_MANAGER: RwLock<IRQManager> = RwLock::new(IRQManager::new());
}

/// Register an IRQ handler according to `irq_info` (eg. MSIX or MSI)
pub fn register_irq(
    irq_info: IRQInfo,
    triggering: IntTrigger,
    handler: &'static dyn InterruptReceiver,
) {
    IRQ_MANAGER
        .write()
        .register_irq(irq_info, triggering, handler);
}
