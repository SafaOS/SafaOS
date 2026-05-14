use crate::{
    PhysAddr,
    arch::aarch64::cpu::CPUDevice,
    drivers::{interrupts::IntTrigger, pci::PCI},
    info,
    memory::{
        AlignToPage,
        paging::PAGE_SIZE,
        vmm::{self, VMMMFlags},
    },
    utils::locks::LazyLock,
};

use hfdt_rs::{self as dtb, Cells};

struct PCIe {
    phys_addr: PhysAddr,
    size: usize,
    bus_start: u32,
    bus_end: u32,
    interrupt_map_mask: Option<[u32; 4]>,
    node: dtb::Node<'static>,
}

impl PCIe {
    fn interrupt_map(
        &self,
    ) -> Option<impl Iterator<Item = ([u32; 3], u32, dtb::Node<'static>, Cells<'static>)>> {
        self.node.parse_interrupt_map().map(|map| {
            map.map(|(mut addr_cells, mut pin_num, node, _, int_spec)| {
                (
                    addr_cells
                        .next_array::<3>()
                        .expect("PCI address cells must be 3"),
                    pin_num.next_cell().unwrap(),
                    node,
                    int_spec,
                )
            })
        })
    }

    /// Looks up the interrupt map entry for the given PCI bus, slot, function, and pin.
    pub fn lookup_int_map(
        &self,
        bus: u32,
        slot: u32,
        function: u32,
        pin: u32,
    ) -> Option<(dtb::Node<'static>, Cells<'static>)> {
        let mask = self.interrupt_map_mask?;
        let lookup_value = [((bus << 16) | (slot << 11) | (function << 8)), 0, 0, pin];
        let masked_lookup_value = [
            lookup_value[0] & mask[0],
            lookup_value[1] & mask[1],
            lookup_value[2] & mask[2],
            lookup_value[3] & mask[3],
        ];

        self.interrupt_map()?
            .find(|(addr, pin, _, _)| {
                addr[0] == masked_lookup_value[0]
                    && addr[1] == masked_lookup_value[1]
                    && addr[2] == masked_lookup_value[2]
                    && *pin == masked_lookup_value[3]
            })
            .map(|(_, _, node, spec)| (node, spec))
    }
}

impl CPUDevice for PCIe {
    const COMPATIBLE: &'static [&'static str] = &["pci-host-ecam-generic"];

    fn create(node: dtb::Node<'static>) -> Result<Self, &'static str> {
        let (phys_addr, size) = node
            .reg_addresses()
            .ok_or("Invalid <reg> for PCIe")?
            .map(|(addr, size)| (PhysAddr::from(addr), size))
            .next()
            .ok_or("No <reg> address for PCIe")?;

        let [bus_start, bus_end] = node
            .property("bus-range")
            .and_then(|p| p.as_cells().next_array::<2>())
            .unwrap_or([0, 255]);

        Ok(Self {
            phys_addr,
            size,
            bus_start,
            bus_end,
            interrupt_map_mask: node
                .interrupt_map_mask()
                .and_then(|mut map_mask| map_mask.next_array()),
            node,
        })
    }
}

static PCIE: LazyLock<Option<PCIe>> = LazyLock::new(|| PCIe::lookup());

/// Lookup the given interrupt pin mapping to the corresponding interrupt controller for the given PCIe device.
pub fn lookup_int_map(
    bus: u32,
    slot: u32,
    function: u32,
    pin: u32,
) -> Option<(dtb::Node<'static>, Cells<'static>)> {
    let pcie = PCIE.as_ref()?;
    pcie.lookup_int_map(bus, slot, function, pin)
}

pub fn init() -> Option<PCI> {
    let pcie = PCIE.as_ref()?;

    let bus_start = pcie.bus_start;
    let bus_end = pcie.bus_end;
    let size = pcie.size;
    let start_phys_addr = pcie.phys_addr;

    info!("initializing PCI from bus: {bus_start:#x} to bus: {bus_end:#x}");

    let page_num = size.to_next_page() / PAGE_SIZE;

    let virt_addr = vmm::with_root(|vmm| {
        vmm.map_direct_phys(
            &"PCIE",
            None,
            start_phys_addr,
            page_num,
            VMMMFlags::WRITEABLE | VMMMFlags::UNCACHABLE,
        )
    })
    .expect("Failed to map memory for the PCIE");

    info!("mapped PCIe from {virt_addr:#x} with size {size:#x}");
    // FIXME: hardcoded bus numbers
    Some(PCI::new(virt_addr, bus_start as u8, bus_end as u8))
}

pub fn build_msi_data(vector: u32, trigger: IntTrigger) -> u32 {
    _ = trigger;
    vector
}
pub fn build_msi_addr() -> PhysAddr {
    super::gic::its::gits_translater_phys()
}
