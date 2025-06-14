use bitfield_struct::bitfield;

use crate::PhysAddr;

pub const TRB_TYPE_LINK: u8 = 0x6;
pub const TRB_TYPE_ENABLE_SLOT_CMD: u8 = 0x9;
pub const TRB_TYPE_TRANSFER_EVENT: u8 = 0x20;
pub const TRB_TYPE_CMD_COMPLETION: u8 = 0x21;
pub const TRB_TYPE_PORT_STATUS_CHANGE_EVENT: u8 = 0x22;

#[bitfield(u32)]
pub struct TRBCommand {
    #[bits(1)]
    pub cycle_bit: u8,
    #[bits(1)]
    pub toggle_cycle: bool,
    __: u8,
    #[bits(6)]
    pub trb_type: u8,
    __: u16,
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct TRB {
    pub parameter: u64,
    pub status: u32,
    pub cmd: TRBCommand,
}

impl TRB {
    pub fn new(cmd: TRBCommand, status: u32, parameter: u64) -> Self {
        Self {
            parameter,
            status,
            cmd,
        }
    }

    /// Attempts to convert self into a known Event Response TRB, returns None if failed
    pub fn into_event_trb(self) -> Option<EventResponseTRB> {
        macro_rules! decided {
            ($variant: ident) => {
                Some(EventResponseTRB::$variant(unsafe {
                    core::mem::transmute(self)
                }))
            };
        }
        match self.cmd.trb_type() {
            TRB_TYPE_CMD_COMPLETION => decided!(CommandCompletion),
            TRB_TYPE_TRANSFER_EVENT => decided!(TransferResponse),
            TRB_TYPE_PORT_STATUS_CHANGE_EVENT => decided!(PortStatusChange),
            _ => None,
        }
    }
}

pub enum EventResponseTRB {
    CommandCompletion(CmdResponseTRB),
    TransferResponse(TransferResponseTRB),
    PortStatusChange(PortStatusChangeTRB),
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum CompletionStatusCode {
    Invalid = 0,
    Success = 1,
    DataBufferErr = 2,
    BabbleDetectedErr = 3,
    UsbTransactionErr = 4,
    TrbErr = 5,
    StallErr = 6,
    ResourceErr = 7,
    BandwidthErr = 8,
    NoSlotsAvailable = 9,
    InvalidStreamType = 0xA,
    SlotNotEnabled = 0xB,
    EndpointNotEnabled = 0xC,
    ShortPacket = 0xD,
    RingUnderrun = 0xE,
    RingOverrun = 0xF,
    VFEventRingFull = 0x10,
    ParameterErr = 0x11,
    BandwidthOverrun = 0x12,
    ContextStateErr = 0x13,
    NoPingResponse = 0x14,
    EventRingFull = 0x15,
    IncompatibleDevice = 0x16,
    MissedService = 0x17,
    CommandRingStopped = 0x18,
    CommandAborted = 0x19,
    Stopped = 0x1A,
    StoppedLengthInvalid = 0x1B,
    StoppedShortPacket = 0x1C,
    MaxExitLatencyErr = 0x1D,
    Other,
}

impl CompletionStatusCode {
    pub const fn from_bits(bits: u8) -> Self {
        if bits >= Self::Other as u8 {
            Self::Other
        } else {
            unsafe { core::mem::transmute(bits) }
        }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

#[bitfield(u32)]
pub struct CmdCompletionStatus {
    #[bits(24)]
    __: (),
    #[bits(8)]
    pub code: CompletionStatusCode,
}

#[bitfield(u32)]
pub struct CmdComplInfo {
    #[bits(1)]
    pub cycle_bit: u8,
    #[bits(9)]
    __rsdv1: (),
    #[bits(6)]
    pub trb_type: u8,
    pub vfid: u8,
    pub slot_id: u8,
}

/// Command Completion TRB Event
#[derive(Debug, Clone)]
#[repr(C)]
pub struct CmdResponseTRB {
    pub trb_pointer: PhysAddr,
    pub status: CmdCompletionStatus,
    pub cmd: CmdComplInfo,
}

#[bitfield(u32)]
pub struct TransferResponseInfo {
    #[bits(1)]
    pub cycle_bit: u8,
    pub eval_next_trb: bool,
    pub interrupt_on_short_pkt: bool,
    pub no_snoop: bool,
    pub chain_bit: bool,
    pub interrupt_on_completion: bool,
    pub immediate_data: bool,
    #[bits(2)]
    __: (),
    pub block_event_interrupt: bool,
    #[bits(6)]
    pub trb_type: u8,
    __: u16,
}

#[derive(Debug)]
#[repr(C)]
pub struct TransferResponseTRB {
    pub parameter: u64,
    pub status: u32,
    pub cmd: TransferResponseInfo,
}

#[bitfield(u64)]
pub struct PortStatusChangePar {
    #[bits(24)]
    __: (),
    port_id: u8,
    __: u32,
}
impl PortStatusChangePar {
    /// Returns the port_id - 1
    pub fn port_index(&self) -> u8 {
        self.port_id() - 1
    }
}

#[bitfield(u32)]
pub struct PortStatusChangeStatus {
    #[bits(24)]
    __: (),
    #[bits(8)]
    pub completion_code: CompletionStatusCode,
}

#[bitfield(u32)]
pub struct PortStatusChangeInfo {
    #[bits(1)]
    pub cycle_bit: u8,
    #[bits(9)]
    __: (),
    #[bits(6)]
    pub trb_type: u8,
    __: u16,
}

#[derive(Debug)]
#[repr(C)]
pub struct PortStatusChangeTRB {
    pub parameter: PortStatusChangePar,
    pub status: PortStatusChangeStatus,
    pub cmd: PortStatusChangeInfo,
}
