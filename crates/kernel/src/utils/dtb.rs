//! Wrapper and helper around device trees (DTB)

use core::fmt::Display;
use core::fmt::Write;
use core::str::FromStr;

use hermit_dtb::Dtb;
use hermit_dtb::EnumPropertiesIter;
use hermit_dtb::EnumSubnodesIter;

use crate::PhysAddr;

/// The [`reg`](https://devicetree-specification.readthedocs.io/en/latest/chapter2-devicetree-basics.html#reg) property of a Device Tree Node
pub struct NodeRegProp<'a> {
    items: &'a [u32],
    address_cells: u32,
    size_cells: u32,
}

impl<'a> NodeRegProp<'a> {
    fn new(items: &'a [u32], address_cells: u32, size_cells: u32) -> Self {
        assert!(address_cells <= 2);
        assert!(size_cells <= 2);
        Self {
            items,
            address_cells,
            size_cells,
        }
    }

    /// Panicks if the amount of bytes isn't a multiply of 4 or unaligned to 4
    fn from_bytes(bytes: &'a [u8], address_cells: u32, size_cells: u32) -> Self {
        let ([], u32s, []) = (unsafe { bytes.align_to::<u32>() }) else {
            panic!("bytes unaligned");
        };

        Self::new(u32s, address_cells, size_cells)
    }
}

impl<'a> Iterator for NodeRegProp<'a> {
    type Item = (PhysAddr, usize);
    fn next(&mut self) -> Option<Self::Item> {
        if self.items.is_empty() {
            return None;
        }

        let total = (self.address_cells + self.size_cells) as usize;
        let mut item = self.items[..total].into_iter();
        self.items = &self.items[total..];

        let addr = if self.address_cells == 2 {
            let higher = u32::from_be(*item.next().unwrap());
            let lower = u32::from_be(*item.next().unwrap());
            (higher as usize) << 32u8 | (lower as usize)
        } else {
            assert_eq!(self.size_cells, 1);
            u32::from_be(*item.next().unwrap()) as usize
        };

        let size = if self.size_cells == 2 {
            let higher = u32::from_be(*item.next().unwrap());
            let lower = u32::from_be(*item.next().unwrap());
            (higher as usize) << 32u8 | (lower as usize)
        } else {
            assert_eq!(self.size_cells, 1);
            u32::from_be(*item.next().unwrap()) as usize
        };

        Some((addr, size))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StrList<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> StrList<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }
}

impl<'a> Iterator for StrList<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.bytes.len() {
            return None;
        }

        let begin = self.index;
        while self.bytes.get(self.index).is_some_and(|b| *b != b'\0') {
            self.index += 1;
        }

        let str = unsafe { str::from_utf8_unchecked(&self.bytes[begin..self.index]) };
        self.index += 1;
        Some(str)
    }
}

#[derive(PartialEq, Eq, Clone)]
pub enum NodeValue<'a> {
    Str(&'a str),
    StrList(StrList<'a>),
    U32(u32),
    Other(&'a [u8]),
    Empty,
}

impl<'a> Display for NodeValue<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::U32(u) => write!(f, "<{u:#x}>")?,
            Self::Str(s) => write!(f, "\"{s}\"")?,
            Self::StrList(sl) => {
                let sl = sl.clone();
                for s in sl {
                    write!(f, "\"{s}\" ")?;
                }
            }
            Self::Other(ref prop_encoded) => {
                let (start, prop_encoded_mid, reset) = unsafe { prop_encoded.align_to::<u32>() };
                write!(f, "<")?;

                for b in start {
                    let b = b.to_ne_bytes();
                    let b = u8::from_be_bytes(b);
                    write!(f, " {b:#x}")?;
                }

                for b in prop_encoded_mid {
                    let b = b.to_ne_bytes();
                    let b = u32::from_be_bytes(b);
                    write!(f, " {b:#x}")?;
                }

                for b in reset {
                    let b = b.to_ne_bytes();
                    let b = u8::from_be_bytes(b);
                    write!(f, " {b:#x}")?;
                }

                write!(f, " >")?;
            }
            Self::Empty => write!(f, "<()>")?,
        }
        Ok(())
    }
}

/// An iterator over the subnodes of a device tree Node
pub struct EnumSubNodes<'a, 'b> {
    node: &'b Node<'a>,
    inner: EnumSubnodesIter<'a, 'b>,
}

impl<'a, 'b> EnumSubNodes<'a, 'b> {
    fn new(node: &'b Node<'a>, inner: EnumSubnodesIter<'a, 'b>) -> Self {
        Self { node, inner }
    }

    fn next(&mut self) -> Option<Node<'a>> {
        let next_node = self.inner.next()?;
        let mut node_path = heapless::String::new();
        write!(node_path, "{}{next_node}/", self.node.path).expect("node path too long");
        Some(Node::new(self.node.parent, node_path))
    }
}

impl<'a, 'b> Iterator for EnumSubNodes<'a, 'b> {
    type Item = Node<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        EnumSubNodes::next(self)
    }
}

/// An Iterator over the properties of a device tree Node
pub struct EnumNodeProps<'a, 'b> {
    node: &'b Node<'a>,
    inner: EnumPropertiesIter<'a, 'b>,
}

impl<'a, 'b> EnumNodeProps<'a, 'b> {
    fn new(node: &'b Node<'a>, inner: EnumPropertiesIter<'a, 'b>) -> Self {
        Self { node, inner }
    }
}

impl<'a, 'b> Iterator for EnumNodeProps<'a, 'b> {
    type Item = (&'b str, NodeValue<'b>);
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.next()?;
        let value = self.node.get_prop(next).unwrap();
        Some((next, value))
    }
}

/// A device tree's sub Node
#[derive(Clone)]
pub struct Node<'a> {
    path: heapless::String<128>,
    parent: &'a DeviceTree<'a>,
}

/// Gets the property of a Node and assumes it is of a certain kind
macro_rules! node_get_prop_unchecked {
    ($node: expr, $name: literal, $kind: path) => {{
        let prop = $node.get_prop($name);
        prop.map(|prop| {
            let $kind(x) = prop else {
                unreachable!();
            };
            x
        })
    }};
    ($node: expr, $name: literal) => {{
        let prop = $node.get_prop($name)?;
        let NodeValue::Other(x) = prop else {
            unreachable!();
        };
        Some(x)
    }};
}

impl<'a> Node<'a> {
    fn name(&self) -> &str {
        let mut path_spilt = self.path.split('/');
        let mut name = "";
        while let Some(part) = path_spilt.next_back() {
            if !part.is_empty() {
                name = part;
                break;
            }
        }
        name
    }
    fn new(parent: &'a DeviceTree<'a>, path: heapless::String<128>) -> Self {
        Self { parent, path }
    }

    /// Returns the value of the property named `name` in the node
    pub fn get_prop(&self, name: &str) -> Option<NodeValue> {
        let property = self.parent.inner.get_property(&self.path, name)?;

        Some(match name {
            "phandle" | "#address-cells" | "#size-cells" | "#interrupt-cells" | "virtual-reg"
            | "interrupt-parent" => {
                let property = property.as_array::<4>().unwrap();
                NodeValue::U32(u32::from_be_bytes(*property))
            }
            "compatible" => NodeValue::StrList(StrList::new(property)),
            "status" | "model" => NodeValue::Str(unsafe { str::from_utf8_unchecked(property) }),
            _ => {
                if property.is_empty() {
                    NodeValue::Empty
                } else {
                    NodeValue::Other(property)
                }
            }
        })
    }

    fn get_compatible(&'a self) -> Option<StrList<'a>> {
        let prop = self.get_prop("compatible")?;
        let NodeValue::StrList(list) = prop else {
            unreachable!();
        };
        Some(list)
    }

    /// Returns whether or not the node is describes a device compatible with any items in the list `compatible_list`
    pub fn is_compatible(&self, compatible_list: &[&str]) -> bool {
        let Some(compatibles) = self.get_compatible() else {
            return false;
        };

        for compatible in compatibles {
            if compatible_list.contains(&compatible) {
                return true;
            }
        }

        false
    }

    /// Gets the `reg` property from the node if available
    pub fn get_reg(&self) -> Option<NodeRegProp> {
        let bytes = node_get_prop_unchecked!(self, "reg")?;
        let address_cells =
            node_get_prop_unchecked!(self, "#address-cells", NodeValue::U32).unwrap_or(2);
        let size_cells = node_get_prop_unchecked!(self, "#size-cells", NodeValue::U32).unwrap_or(1);

        Some(NodeRegProp::from_bytes(bytes, address_cells, size_cells))
    }

    pub fn subnodes<'b>(&'b self) -> EnumSubNodes<'a, 'b> {
        let inner = self.parent.inner.enum_subnodes(&self.path);
        EnumSubNodes::new(self, inner)
    }

    pub fn properties<'b>(&'b self) -> EnumNodeProps<'a, 'b> {
        let inner = self.parent.inner.enum_properties(&self.path);
        EnumNodeProps::new(self, inner)
    }

    pub fn fmt(&self, f: &mut core::fmt::Formatter, depth: usize) -> core::fmt::Result {
        macro_rules! wr {
            ($($arg:tt)*) => {
                writeln!(f, "{:depth$}{}", "", format_args!($($arg)*))
            };
        }

        wr!("{}{{", self.name())?;
        for (name, value) in self.properties() {
            wr!("   {name}: {value};")?;
        }

        for node in self.subnodes() {
            node.fmt(f, depth + 3)?;
        }

        wr!("}}")?;

        Ok(())
    }
}

impl<'a> Display for Node<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.fmt(f, 0)
    }
}

/// Wrapper around a (DTB Blob)
pub struct DeviceTree<'a> {
    // TODO: use own implementition
    inner: Dtb<'a>,
}

impl<'a> DeviceTree<'a> {
    unsafe fn from_ptr(ptr: *const ()) -> Result<Self, ()> {
        unsafe {
            Dtb::from_raw(ptr as *const u8)
                .map(|inner| Self { inner })
                .ok_or(())
        }
    }

    pub fn retrieve_from_limine() -> Option<Self> {
        let addr = crate::limine::device_tree_addr();
        addr.map(|ptr| unsafe {
            Self::from_ptr(ptr).expect("limine's reply's Device Tree Blob is invalid")
        })
    }
    /// Returns the root node of the Device Tree ("/")
    pub fn root_node<'n>(&'n self) -> Node<'n> {
        Node::new(self, heapless::String::from_str("/").unwrap())
    }
}

impl<'a> Display for DeviceTree<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.root_node().fmt(f, 0)
    }
}
