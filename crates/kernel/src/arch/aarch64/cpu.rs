//! CPU sepicific stuff
//! uses device trees only for now

use core::{cell::SyncUnsafeCell, mem::zeroed};

use lazy_static::lazy_static;

use crate::{
    utils::dtb::{self, DeviceTree, NodeValue},
    PhysAddr,
};

struct GICInfo {
    gicc_base: PhysAddr,
    gicd_base: PhysAddr,
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

trait DeviceInfo {
    fn populated(&self) -> bool;
    fn compatible(&self) -> &'static [&'static str];
    /// Populates the Device's information from a Device Tree Node which is compatible with the device (according to [`Self::compatible`])
    fn populate<'a>(&mut self, node: dtb::Node);
}

impl DeviceInfo for GICInfo {
    fn populated(&self) -> bool {
        self.populated
    }

    fn compatible(&self) -> &'static [&'static str] {
        &["arm,cortex-a15-gic"]
    }

    fn populate<'a>(&mut self, node: dtb::Node) {
        let mut reg = node.get_reg().unwrap();
        let (gicc_base, _) = reg.next().unwrap();
        let (gicd_base, _) = reg.next().unwrap();

        self.gicc_base = gicc_base;
        self.gicd_base = gicd_base;
        self.populated = true;
    }
}

impl DeviceInfo for TimerInfo {
    fn populated(&self) -> bool {
        self.populated
    }

    fn populate<'a>(&mut self, node: dtb::Node) {
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
        let Some([ty, int_id, flags]) = interrupts.next() else {
            unreachable!()
        };

        // makes sure it is PPI
        assert_eq!(ty, 0x1);
        // makes sure it is at CPU 0
        assert_eq!((flags >> 8) as u8, 0b00000001);
        let irq = int_id + 0x10;

        self.irq = irq;
        self.populated = true;
    }
    fn compatible(&self) -> &'static [&'static str] {
        &["arm,armv8-timer"]
    }
}

impl DeviceInfo for PL011Serial {
    fn compatible(&self) -> &'static [&'static str] {
        &["arm,pl011"]
    }

    fn populated(&self) -> bool {
        self.populated
    }

    fn populate<'a>(&mut self, node: dtb::Node) {
        let mut reg = node.get_reg().unwrap();
        let (addr, _) = reg.next().unwrap();
        self.base = addr;
        self.populated = true;
    }
}

static GICRAW: SyncUnsafeCell<GICInfo> = SyncUnsafeCell::new(unsafe { zeroed() });
static TIMERRAW: SyncUnsafeCell<TimerInfo> = SyncUnsafeCell::new(unsafe { zeroed() });
static PL011RAW: SyncUnsafeCell<PL011Serial> = SyncUnsafeCell::new(unsafe { zeroed() });

const unsafe fn devices() -> [&'static mut dyn DeviceInfo; 3] {
    let gic = &mut *GICRAW.get();
    let gic: &'static mut dyn DeviceInfo = gic;
    unsafe { [gic, &mut *TIMERRAW.get(), &mut *PL011RAW.get()] }
}

fn init_from_tree(tree: &DeviceTree) {
    let root = tree.root_node();

    // list of devices requirng initialization
    let mut devices = unsafe { devices() };

    fn handle_node<'a>(node: dtb::Node<'a>, devices: &mut [&mut dyn DeviceInfo]) {
        for device in &mut *devices {
            if !device.populated() {
                if node.is_compatible(device.compatible()) {
                    device.populate(node);
                    // TODO: for now we can just stop when a node is proven to be compatible
                    return;
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
    pub static ref GIC: (PhysAddr, PhysAddr) = unsafe {
        let r = &mut *GICRAW.get();
        if !r.populated() {
            init();
        }
        (r.gicc_base, r.gicd_base)
    };
}
