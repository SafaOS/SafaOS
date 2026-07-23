use lazy_static::lazy_static;

use super::acpi;
use crate::{
    PhysAddr,
    arch::x86_64::{acpi::MCFGEntry, interrupts::apic::APIC},
    drivers::{interrupts::IntTrigger, pci::PCI},
    memory::vmm::{self, VMMMFlags},
};

lazy_static! {
    pub static ref PCI_MCFG_ENTRY: Option<MCFGEntry> = {
        let mcfg = (*acpi::MCFG_DESC)?;
        let entry = mcfg.nth(0);
        entry
    };
}

pub fn init() -> Option<PCI> {
    if let Some(entry) = *PCI_MCFG_ENTRY {
        assert_eq!(entry.pci_sgn, 0);

        let flags = VMMMFlags::WRITEABLE | VMMMFlags::UNCACHABLE;

        let pci_phys = entry.physical_addr;
        // bus count * slot count * 4096 = size
        // page num = size / 4096
        let pci_page_num = (entry.pci_num1 - entry.pci_num0) as usize * 256;

        let virt_addr = vmm::with_root(|vmm| {
            vmm.map_direct_phys(
                &"PCIE",
                // TODO: maybe make it so the VirtAddr is dynamic?
                None,
                pci_phys,
                pci_page_num,
                flags,
            )
        })
        .expect("Failed to allocate space for the PCIE");
        Some(PCI::new(virt_addr, entry.pci_num0, entry.pci_num1))
    } else {
        None
    }
}

pub fn build_msi_data(irq_num: u32, trigger: IntTrigger) -> u32 {
    let (trigger, assert) = match trigger {
        IntTrigger::Edge => (0, 0),
        IntTrigger::LevelDeassert => (1, 0),
        IntTrigger::LevelAssert => (1, 1),
    };

    let results = irq_num | /* TODO: Delivery */ 0 | assert << 14 | trigger << 15;
    results
}
pub fn build_msi_addr(target_cpu: crate::percpu::CpuID) -> PhysAddr {
    let cpu = crate::percpu::CpuLocal::get_for(target_cpu);
    let lapic_base = APIC.lapic_base().into_raw();
    let lapic_id = cpu.cpu_arch_id.lapic_id();
    let msi_addr = lapic_base | ((lapic_id as usize) << 12);
    PhysAddr::from(msi_addr)
}
