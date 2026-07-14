//! CPU sepicific stuff
//! uses device trees only for now
// FIXME: incomplete and code is bad, i need to rework this in the future
use core::str::FromStr;

use crate::limine::FDT;
use crate::utils::locks::LazyLock;
use crate::warn;

use hfdt_rs::{self as dtb};

/// Represents a CPU Device that can be retrieved from a Device Tree.
pub trait CPUDevice: Sized {
    /// A list of <compatible> strings that would work with this Device.
    const COMPATIBLE: &'static [&'static str];
    /// Constructs a new CPU Device from a compatible Node.
    fn create(node: dtb::Node<'static>) -> Result<Self, &'static str>;
    /// Returns true if the Node is compatible with this CPU Device, which would make [`Self::lookup`] use it to create this device.
    fn node_matches(node: &dtb::Node) -> bool {
        node.compatible()
            .is_some_and(|mut c| c.any(|s| Self::COMPATIBLE.contains(&s)))
    }
    /// Lookups this Device in the DBT and then attempts to `Self::create`s it.
    fn lookup() -> Option<Self>
    where
        Self: Sized,
    {
        cpu_tree_lookup(Self::node_matches)
            .map(|n| {
                let r = Self::create(n.clone());
                if let Err(e) = r {
                    warn!("CPU Device compatible with: {:?}, node found but device creation failed\n++++++++++++ NODE ++++++++++++\n{n}\n============ NODE ============\nError: {e}", Self::COMPATIBLE);
                }
                r.ok()
            })
            .flatten()
    }
}

/// Lookup a Device tree node that the given function returns true on.
pub fn cpu_tree_lookup(lookup_fn: impl Fn(&dtb::Node) -> bool) -> Option<dtb::Node<'static>> {
    FDT.find_node(lookup_fn)
}

struct CPURoot {
    model: heapless::String<48>,
}

impl CPUDevice for CPURoot {
    const COMPATIBLE: &'static [&'static str] = &[];
    fn node_matches(node: &dtb::Node) -> bool {
        // Only matches the root node.
        node.name() == ""
    }
    fn create(node: dtb::Node) -> Result<Self, &'static str> {
        let model = node
            .property("model")
            .and_then(|model| model.as_str())
            .ok_or("CPU Model missing")?;
        Ok(Self {
            model: heapless::String::from_str(&model[..model.len().min(48)]).unwrap(),
        })
    }
}

static CPU_ROOT: LazyLock<Option<CPURoot>> = LazyLock::new(|| CPURoot::lookup());

/// Returns the name of the CPU's model.
pub fn cpu_model() -> &'static str {
    CPU_ROOT
        .as_ref()
        .map(|m| m.model.as_str())
        .unwrap_or("UNKNOWN")
}
