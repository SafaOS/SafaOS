use super::handlers::IDT;
use lazy_static::lazy_static;

#[allow(clippy::upper_case_acronyms)]
pub type IDTT = [GateDescriptor; 256];

#[repr(C, packed)]
pub struct IDTDescriptor {
    limit: u16,
    base: usize,
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct GateDescriptor {
    offset0: u16,
    selector: u16,
    pub ist: u8,
    attributes: u8, // gate_type, dpl, zero and present bit
    offset1: u16,
    offset2: u32,
    reserved: u32,
}

impl GateDescriptor {
    pub const fn new(handler: usize, attributes: u8) -> Self {
        let offset = handler;
        Self {
            offset0: offset as u16,
            selector: 0x08,
            ist: 0,
            attributes: attributes | 1 << 7, // attaching present attriubute
            offset1: (offset >> 16) as u16,
            offset2: (offset >> 32) as u32,
            reserved: 0,
        }
    }

    pub const fn default() -> Self {
        Self {
            offset0: 0,
            selector: 0,
            ist: 0,
            attributes: 0,
            offset1: 0,
            offset2: 0,
            reserved: 0,
        }
    }
}

lazy_static! {
    pub static ref IDTDesc: IDTDescriptor = IDTDescriptor {
        limit: (size_of::<IDTT>() - 1) as u16,
        base: IDT.as_ptr() as usize
    };
}
