//! CPU sepicific stuff
//! uses device trees only for now
// FIXME: incomplete and code is bad, i need to rework this in the future
use core::str::FromStr;
use core::{cell::SyncUnsafeCell, mem::zeroed};
use lazy_static::lazy_static;

use crate::{
    utils::dtb::{self, DeviceTree, NodeValue},
    PhysAddr,
};

struct GICInfo {
    gicc: Option<(PhysAddr, usize)>,
    gicd: (PhysAddr, usize),
    gicr: (PhysAddr, usize),
    populated: bool,
}

struct ITSInfo {
    base: PhysAddr,
    size: usize,
    populated: bool,
}

struct TimerInfo {
    irq: u32,
    populated: bool,
}

struct PL011Serial {
    base: PhysAddr,
    populated: bool,
}

struct PCIe {
    base: PhysAddr,
    size: usize,
    bus_start: u32,
    bus_end: u32,
    populated: bool,
}

trait DeviceInfo {
    fn populated(&self) -> bool;
    fn compatible(&self) -> &'static [&'static str];
    /// Populates the Device's information from a Device Tree Node which is compatible with the device (according to [`Self::compatible`])
    fn populate<'a>(&mut self, node: &dtb::Node);
}

impl DeviceInfo for GICInfo {
    fn populated(&self) -> bool {
        self.populated
    }

    fn compatible(&self) -> &'static [&'static str] {
        &["arm,gic-v3"]
    }

    fn populate<'a>(&mut self, node: &dtb::Node) {
        let mut reg = node.get_reg().unwrap();
        self.gicd = reg.next().unwrap();
        self.gicr = reg.next().unwrap();
        self.gicc = reg.next();

        self.populated = true;
    }
}

impl DeviceInfo for ITSInfo {
    fn populated(&self) -> bool {
        self.populated
    }

    fn compatible(&self) -> &'static [&'static str] {
        &["arm,gic-v3-its"]
    }

    fn populate<'a>(&mut self, node: &dtb::Node) {
        let mut reg = node.get_reg().unwrap();
        let (base_addr, size) = reg.next().unwrap();

        self.base = base_addr;
        self.size = size;
        self.populated = true;
    }
}

impl DeviceInfo for TimerInfo {
    fn populated(&self) -> bool {
        self.populated
    }

    fn populate<'a>(&mut self, node: &dtb::Node) {
        let interrupts = node
            .get_prop("interrupts")
            .expect("failed to get the interrupts property for the timer's device tree node");

        let NodeValue::Other(interrupts_bytes) = interrupts else {
            unreachable!()
        };
        let ([], interrupts_u32s, []) = (unsafe { interrupts_bytes.align_to::<u32>() }) else {
            unreachable!()
        };

        let interrupts = interrupts_u32s.into_iter().map(|u| u32::from_be(*u));
        let mut interrupts = interrupts.array_chunks::<3>();

        let _secure = interrupts.next();
        // non-secure physical timer interrupt
        let Some([ty, int_id, _]) = interrupts.next() else {
            unreachable!()
        };

        // makes sure it is PPI
        assert_eq!(ty, 0x1);
        let irq = int_id + 0x10;

        self.irq = irq;
        self.populated = true;
    }
    fn compatible(&self) -> &'static [&'static str] {
        &["arm,armv8-timer", "arm,armv7-timer"]
    }
}

impl DeviceInfo for PL011Serial {
    fn compatible(&self) -> &'static [&'static str] {
        &["arm,pl011"]
    }

    fn populated(&self) -> bool {
        self.populated
    }

    fn populate<'a>(&mut self, node: &dtb::Node) {
        let mut reg = node.get_reg().unwrap();
        let (addr, _) = reg.next().unwrap();
        self.base = addr;
        self.populated = true;
    }
}

impl DeviceInfo for PCIe {
    fn compatible(&self) -> &'static [&'static str] {
        &["pci-host-ecam-generic"]
    }
    fn populated(&self) -> bool {
        self.populated
    }
    fn populate<'a>(&mut self, node: &dtb::Node) {
        let mut reg = node.get_reg_no_cells().unwrap();
        let (start, size) = reg.next().unwrap();

        let Some(NodeValue::Other(bytes)) = node.get_prop("bus-range") else {
            unreachable!()
        };

        let bus_start_bytes = bytes[..4].as_array::<4>().unwrap();
        let bus_end_bytes = bytes[4..8].as_array::<4>().unwrap();
        let bus_start = u32::from_be_bytes(*bus_start_bytes);
        let bus_end = u32::from_be_bytes(*bus_end_bytes);

        self.bus_start = bus_start;
        self.bus_end = bus_end;

        self.base = start;
        self.size = size;
        self.populated = true;
    }
}

static GICRAW: SyncUnsafeCell<GICInfo> = SyncUnsafeCell::new(unsafe { zeroed() });
static ITSRAW: SyncUnsafeCell<ITSInfo> = SyncUnsafeCell::new(unsafe { zeroed() });
static TIMERRAW: SyncUnsafeCell<TimerInfo> = SyncUnsafeCell::new(unsafe { zeroed() });
static PL011RAW: SyncUnsafeCell<PL011Serial> = SyncUnsafeCell::new(unsafe { zeroed() });
static PCIERAW: SyncUnsafeCell<PCIe> = SyncUnsafeCell::new(unsafe { zeroed() });

const unsafe fn devices() -> [&'static mut dyn DeviceInfo; 5] {
    let gic = &mut *GICRAW.get();
    let gic: &'static mut dyn DeviceInfo = gic;
    unsafe {
        [
            gic,
            &mut *ITSRAW.get(),
            &mut *TIMERRAW.get(),
            &mut *PL011RAW.get(),
            &mut *PCIERAW.get(),
        ]
    }
}

fn init_from_tree(tree: &DeviceTree) {
    let root = tree.root_node();
    let s = root.get_model().unwrap_or("UNKNOWN");
    unsafe {
        MODEL
            .get()
            .write_volatile(heapless::String::from_str(s).unwrap());
    }

    // list of devices requirng initialization
    let mut devices = unsafe { devices() };

    fn handle_node<'a>(node: dtb::Node<'a>, devices: &mut [&mut dyn DeviceInfo]) {
        for device in &mut *devices {
            if !device.populated() {
                if node.is_compatible(device.compatible()) {
                    device.populate(&node);
                    break;
                }
            }
        }

        for node in node.subnodes() {
            handle_node(node, devices);
        }
    }

    handle_node(root, &mut devices);
    for device in devices {
        assert!(device.populated());
    }
}

/// Initializes CPU specific devices such as the serial
/// NO ALLOCATIONS ALLOWED
pub fn init() {
    let tree = DeviceTree::retrieve_from_limine();
    if let Some(tree) = tree {
        init_from_tree(&tree);
    }
}

/// Returns whether or not the serial device is ready and populated, used for debug puropses (write to qemu's serial if not ready yet)
pub fn serial_ready() -> bool {
    let r = unsafe { &*PL011RAW.get() };
    r.populated
}

pub static MODEL: SyncUnsafeCell<heapless::String<48>> =
    SyncUnsafeCell::new(heapless::String::new());

lazy_static! {
    pub static ref TIMER_IRQ: u32 = unsafe {
        let r = &mut *TIMERRAW.get();
        if !r.populated() {
            init();
        }
        r.irq
    };
    pub static ref PL011BASE: PhysAddr = unsafe {
        let r = &mut *PL011RAW.get();
        if !r.populated() {
            init();
        }
        r.base
    };
    /// The GICv3 registers
    /// Optional (GICC base, GICC size), (GICD base, GICD size), (GICR base, GICR size)
    pub static ref GICV3: (
        Option<(PhysAddr, usize)>,
        (PhysAddr, usize),
        (PhysAddr, usize)
    ) = unsafe {
        let r = &mut *GICRAW.get();
        if !r.populated() {
            init();
        }
        (r.gicc, r.gicd, r.gicr)
    };
    pub static ref GICITS: (PhysAddr, usize) = unsafe {
        let r = &mut *ITSRAW.get();
        if !r.populated() {
            init();
        }
        (r.base, r.size)
    };
    pub static ref PCIE: (PhysAddr, usize, u32, u32) = unsafe {
        let r = &mut *PCIERAW.get();
        if !r.populated() {
            init();
        }
        (r.base, r.size, r.bus_start, r.bus_end)
    };
}
