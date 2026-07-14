use bitfield_struct::bitfield;

use crate::PhysAddr;

pub const TRB_TYPE_NORMAL: u8 = 0x1;
pub const TRB_TYPE_SETUP_STAGE: u8 = 0x2;
pub const TRB_TYPE_DATA_STAGE: u8 = 0x3;
pub const TRB_TYPE_EVENT_DATA: u8 = 0x7;
pub const TRB_TYPE_STATUS_STAGE: u8 = 0x4;

pub const TRB_TYPE_LINK: u8 = 0x6;
pub const TRB_TYPE_ENABLE_SLOT_CMD: u8 = 0x9;
pub const TRB_TYPE_ADDRESS_DEVICE_CMD: u8 = 0xB;
pub const TRB_TYPE_CONFIGURE_ENDPOINT_CMD: u8 = 0xC;
pub const TRB_TYPE_EVALUATE_CONTEXT_CMD: u8 = 0xD;

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

    /// Creates a new link TRB that links to `phys_base_addr`
    pub fn new_link(phys_base_addr: PhysAddr, cycle_bit: u8) -> Self {
        assert!(cycle_bit == 0 || cycle_bit == 1);
        let mut link_trb: Self = unsafe { core::mem::zeroed() };
        link_trb.parameter = phys_base_addr.into_raw() as u64;
        link_trb.cmd.set_trb_type(TRB_TYPE_LINK);
        link_trb.cmd.set_toggle_cycle(true);
        link_trb.cmd.set_cycle_bit(cycle_bit);
        link_trb
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    #[bits(1)]
    __: (),
    pub event_data: bool,
    #[bits(7)]
    __: (),
    #[bits(6)]
    pub trb_type: u8,
    #[bits(5)]
    pub endpoint_id: u8,
    #[bits(3)]
    __: (),
    pub slot_id: u8,
}
#[bitfield(u32)]
pub struct TransferResponseStatus {
    #[bits(24)]
    pub transfer_length: u32,
    #[bits(8)]
    pub completion_code: CompletionStatusCode,
}

#[derive(Debug)]
#[repr(C)]
pub struct TransferResponseTRB {
    pub trb_ptr: PhysAddr,
    pub status: TransferResponseStatus,
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

#[bitfield(u32)]
pub struct AddressDeviceCommandInfo {
    #[bits(1)]
    pub cycle_bit: u8,
    __: u8,

    /// Block Set Address Request (BSR). When this flag is set to ‘0’ the Address Device Command shall
    /// generate a USB SET_ADDRESS request to the device. When this flag is set to ‘1’ the Address
    /// Device Command shall not generate a USB SET_ADDRESS request. Refer to section 4.6.5 for
    /// more information on the use of this flag.
    pub bsr: bool,
    #[bits(6)]
    pub trb_type: u8,
    __: u8,
    pub slot_id: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct AddressDeviceCommandTRB {
    pub input_context_physical_address: PhysAddr,
    __: u32,
    pub info: AddressDeviceCommandInfo,
}

impl AddressDeviceCommandTRB {
    pub const fn new(
        input_context_physical_address: PhysAddr,
        bsr: bool,
        slot_id: u8,
        cycle_bit: u8,
    ) -> Self {
        assert!(cycle_bit == 0 || cycle_bit == 1);

        Self {
            input_context_physical_address,
            __: 0,
            info: AddressDeviceCommandInfo::new()
                .with_bsr(bsr)
                .with_slot_id(slot_id)
                .with_trb_type(TRB_TYPE_ADDRESS_DEVICE_CMD)
                .with_cycle_bit(cycle_bit),
        }
    }

    pub fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }
}

#[bitfield(u32)]
struct ConfigureEndpointCommandTRBCMD {
    #[bits(1)]
    cycle_bit: u8,
    __: u8,
    deconfigure: bool,
    #[bits(6)]
    trb_type: u8,
    __: u8,
    slot_id: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct ConfigureEndpointCommandTRB {
    input_ctx_base: PhysAddr,
    __rsdvz: u32,
    cmd: ConfigureEndpointCommandTRBCMD,
}

impl ConfigureEndpointCommandTRB {
    pub const fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }
    pub const fn new(input_ctx_base: PhysAddr, slot_id: u8) -> Self {
        Self {
            input_ctx_base,
            __rsdvz: 0,
            cmd: ConfigureEndpointCommandTRBCMD::new()
                .with_deconfigure(false)
                .with_cycle_bit(0)
                .with_trb_type(TRB_TYPE_CONFIGURE_ENDPOINT_CMD)
                .with_slot_id(slot_id),
        }
    }
}

#[bitfield(u32)]
pub struct EvaluateContextTRBInfo {
    #[bits(1)]
    cycle_bit: u8,
    #[bits(9)]
    __: (),
    #[bits(6)]
    trb_type: u8,
    __: u8,
    slot_id: u8,
}

#[repr(C)]
pub struct EvaluateContextCMDTRB {
    input_ctx_phys_base: PhysAddr,
    __: u32,
    cmd: EvaluateContextTRBInfo,
}

impl EvaluateContextCMDTRB {
    pub const fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }
    pub const fn new(input_ctx_phys_base: PhysAddr, slot_id: u8) -> Self {
        Self {
            input_ctx_phys_base,
            __: 0,
            cmd: EvaluateContextTRBInfo::new()
                .with_slot_id(slot_id)
                .with_trb_type(TRB_TYPE_EVALUATE_CONTEXT_CMD),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum PacketRecipient {
    Device = 0,
    Interface = 1,
    Endpoint = 2,
    Other = 3,
    Reserved = 4,
}

impl PacketRecipient {
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    pub const fn from_bits(bits: u8) -> Self {
        if bits < Self::Reserved as u8 {
            unsafe { core::mem::transmute(bits) }
        } else {
            Self::Reserved
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum PacketType {
    Standard = 0,
    Class = 1,
    Vendor = 2,
    Reserved = 3,
}

impl PacketType {
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    pub const fn from_bits(bits: u8) -> Self {
        if bits < Self::Reserved as u8 {
            unsafe { core::mem::transmute(bits) }
        } else {
            Self::Reserved
        }
    }
}

/**
// xHci Spec Section 4.11.2.2 Figure 4-14 SETUP Data, the Parameter Component of Setup Stage TRB (page 211)
*/
#[bitfield(u64)]
pub struct XHCIDeviceRequestPacket {
    #[bits(5)]
    pub recipient: PacketRecipient,
    #[bits(2)]
    pub p_type: PacketType,
    pub device_to_host: bool,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

#[bitfield(u32)]
pub struct SetupStageStatus {
    #[bits(17)]
    pub trb_transfer_length: u32,
    #[bits(5)]
    __: (),
    /// This field defines the index of the Interrupter that will receive events
    /// generated by this TRB. Valid values are between 0 and MaxIntrs-1.
    /// TODO: we only use interrupter 0 for now
    #[bits(10)]
    pub interrupter: u16,
}

#[bitfield(u32)]
pub struct SetupStageInfo {
    #[bits(1)]
    /// This bit is used to mark the Enqueue point of a Transfer ring
    pub cycle_bit: u8,
    #[bits(4)]
    __: (),
    /// Interrupt On Completion (IOC). If this bit is set to ‘1’, it specifies that when this TRB
    /// completes, the Host Controller shall notify the system of the completion by placing an
    /// Event TRB on the Event ring and sending an interrupt at the next interrupt threshold.
    /// Refer to section 4.10.4.
    pub ioc: bool,
    /// Immediate Data (IDT). This bit shall be set to ‘1’ in a Setup Stage TRB.
    /// It specifies that the Parameter component of this TRB contains Setup Data.
    pub idt: bool,
    #[bits(3)]
    __: (),
    #[bits(6)]
    pub trb_type: u8,
    /// TODO: make it an enum
    ///
    /// Transfer Type (TRT). This field indicates the type and direction of the control transfer.
    /// Value Definition
    ///
    /// 0 No Data Stage
    ///
    /// 1 Reserved
    ///
    /// 2 OUT Data Stage
    ///
    /// 3 IN Data Stage
    ///
    /// Refer to section 4.11.2.2 for more information on the use of TRT.
    #[bits(2)]
    pub trt: u8,
    #[bits(14)]
    __: (),
}

/**
// xHci Spec Section 6.4.1.2.1 Setup Stage TRB (page 468)

A Setup Stage TRB is created by system software to initiate a USB Setup packet
on a control endpoint. Refer to section 3.2.9 for more information on Setup
Stage TRBs and the operation of control endpoints. Also refer to section 8.5.3 in
the USB2 spec. for a description of “Control Transfers”.
*/
#[repr(C)]
pub struct SetupStageTRB {
    pub parameter: XHCIDeviceRequestPacket,
    pub status: SetupStageStatus,
    pub info: SetupStageInfo,
}

impl SetupStageTRB {
    pub fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }

    pub fn new(packet: XHCIDeviceRequestPacket, interrupter: u16, cycle_bit: u8) -> Self {
        Self {
            parameter: packet,
            status: SetupStageStatus::new().with_interrupter(interrupter),
            info: SetupStageInfo::new()
                .with_idt(true)
                .with_cycle_bit(cycle_bit)
                .with_trb_type(TRB_TYPE_SETUP_STAGE),
        }
    }
}

#[bitfield(u32)]
pub struct DataStagePar {
    /// TRB Transfer Length. For an OUT, this field is the number of data bytes the xHC will send
    /// during the execution of this TRB.
    ///
    /// For an IN, the initial value of the field identifies the size of the data buffer referenced
    /// by the Data Buffer Pointer, i.e. the number of bytes the host expects the endpoint to deliver.
    /// Valid values are 1 to 64K.
    #[bits(17)]
    pub trb_transfer_len: u32,
    /// TD Size. This field provides an indicator of the number of packets remaining in the TD.
    /// Refer to section 4.11.2.4 for how this value is calculated.
    #[bits(5)]
    pub td_size: u8,
    /// This field defines the index of the Interrupter that will receive events
    /// generated by this TRB. Valid values are between 0 and MaxIntrs-1
    ///
    /// TODO: we only use interrupter 0
    #[bits(10)]
    pub interrupter_target: u16,
}

#[bitfield(u32)]
pub struct DataStageCMD {
    #[bits(1)]
    pub cycle_bit: u8,
    /// Evaluate Next TRB (ENT). If this flag is ‘1’ the xHC shall fetch and evaluate the
    /// next TRB before saving the endpoint state. Refer to section 4.12.3 for more information.
    pub ent: bool,
    /// Interrupt-on Short Packet (ISP). If this flag is ‘1’ and a Short Packet is encountered
    /// for this TRB (i.e., less than the amount specified in TRB Transfer Length), then a
    /// Transfer Event TRB shall be generated with its Completion Code set to Short Packet.
    ///
    /// The TRB Transfer Length field in the Transfer Event TRB shall reflect the residual
    /// number of bytes not transferred into the associated data buffer. In either case, when
    /// a Short Packet is encountered, the TRB shall be retired without error and the xHC shall
    /// advance to the Status Stage TD.
    ///
    /// Note: if the ISP and IOC flags are both ‘1’ and a Short Packet is detected, then only one
    /// Transfer Event TRB shall be queued to the Event Ring. Also refer to section 4.10.1.1.
    pub isp: bool,
    /// No Snoop (NS). When set to ‘1’, the xHC is permitted to set the No Snoop bit in the
    /// Requester Attributes of the PCIe transactions it initiates if the PCIe configuration
    /// Enable No Snoop flag is also set.
    ///
    /// When cleared to ‘0’, the xHC is not permitted to set PCIe packet No Snoop Requester
    /// Attribute. Refer to section 4.18.1 for more information.
    ///
    /// NOTE: If software sets this bit, then it is responsible for maintaining cache consistency.
    pub no_snoop: bool,
    /// Chain bit (CH). Set to ‘1’ by software to associate this TRB with the next TRB on the Ring.
    /// A Data Stage TD is defined as a Data Stage TRB followed by zero or more Normal TRBs.
    /// The Chain bit is used to identify a multi-TRB Data Stage TD.
    ///
    /// The Chain bit is always ‘0’ in the last TRB of a Data Stage TD.
    pub chain: bool,
    /// Interrupt On Completion (IOC). If this bit is set to ‘1’, it specifies that when this TRB
    /// completes, the Host Controller shall notify the system of the completion by placing an
    /// Event TRB on the Event ring and sending an interrupt at the next interrupt threshold.
    /// Refer to section 4.10.4.
    pub ioc: bool,
    /// Immediate Data (IDT). This bit shall be set to ‘1’ in a Setup Stage TRB.
    /// It specifies that the Parameter component of this TRB contains Setup Data.
    pub idt: bool,
    #[bits(3)]
    __: (),
    #[bits(6)]
    pub trb_type: u8,
    /// Direction (DIR). This bit indicates the direction of the data transfer as defined in the
    /// Data State TRB Direction column of Table 7. If cleared to ‘0’, the data stage transfer
    /// direction is OUT (Write Data).
    ///
    /// If set to ‘1’, the data stage transfer direction is IN (Read Data).
    ///
    /// Refer to section 4.11.2.2 for more information on the use of DIR.
    pub dir_in: bool,
    #[bits(15)]
    __: (),
}

/**
// xHci Spec Section 6.4.1.2.2 Data Stage TRB Figure 6-10: Data Stage TRB (page 470)

A Data Stage TRB is used generate the Data stage transaction of a USB Control
transfer. Refer to section 3.2.9 for more information on Control transfers and
the operation of control endpoints. Also refer to section 8.5.3 in the USB2 spec.
for a description of “Control Transfers”.
*/
#[repr(C)]
pub struct DataStageTRB {
    /// Data Buffer Pointer Hi and Lo. These fields represent the 64-bit address of the Data
    /// buffer area for this transaction.
    ///
    /// The memory structure referenced by this physical memory pointer is allowed to begin on
    /// a byte address boundary. However, user may find other alignments, such as 64-byte or
    /// 128-byte alignments, to be more efficient and provide better performance
    pub data_buffer_base: PhysAddr,
    pub parameter: DataStagePar,
    pub cmd: DataStageCMD,
}

impl DataStageTRB {
    pub fn new(data_buffer_base: PhysAddr, interrupter: u16) -> Self {
        Self {
            data_buffer_base,
            parameter: DataStagePar::new().with_interrupter_target(interrupter),
            cmd: DataStageCMD::new().with_trb_type(TRB_TYPE_DATA_STAGE),
        }
    }
    pub fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }
}

#[bitfield(u32)]
pub struct EventDataTRBStatus {
    #[bits(22)]
    __: (),
    #[bits(10)]
    pub interrupter_target: u16,
}

#[bitfield(u32)]
pub struct EventDataTRBCmd {
    #[bits(1)]
    pub cycle_bit: u8,
    /// Evaluate Next TRB (ENT). If this flag is ‘1’ the xHC shall fetch and evaluate the
    /// next TRB before saving the endpoint state. Refer to section 4.12.3 for more information.
    pub ent: bool,
    #[bits(2)]
    __: (),
    /// Chain bit (CH). Set to ‘1’ by software to associate this TRB with the next TRB on the Ring.
    /// A Data Stage TD is defined as a Data Stage TRB followed by zero or more Normal TRBs.
    /// The Chain bit is used to identify a multi-TRB Data Stage TD.
    ///
    /// The Chain bit is always ‘0’ in the last TRB of a Data Stage TD.
    pub chain: bool,
    /// Interrupt On Completion (IOC). If this bit is set to ‘1’, it specifies that when this TRB
    /// completes, the Host Controller shall notify the system of the completion by placing an
    /// Event TRB on the Event ring and sending an interrupt at the next interrupt threshold.
    /// Refer to section 4.10.4.
    pub ioc: bool,
    #[bits(3)]
    __: (),
    /// Block Event Interrupt (BEI). If this bit is set to '1' and IOC = '1', then the
    /// Transfer Event generated by IOC shall not assert an interrupt to the host at the
    /// next interrupt threshold. Refer to section 4.17.5.
    pub bei: bool,
    #[bits(6)]
    pub trb_type: u8,
    __: u16,
}

/**
// xHci Spec Section 6.4.4.2 Event Data TRB Figure 6-39: Event Data TRB (page 505).

An Event Data TRB allows system software to generate a software defined event
and specify the Parameter field of the generated Transfer Event.

Note: When applying Event Data TRBs to control transfer: 1) An Event Data TRB may
be inserted at the end of a Data Stage TD in order to report the accumulated
transfer length of a multi-TRB TD. 2) An Event Data TRB may be inserted at the
end of a Status Stage TD in order to provide Event Data associated with the
control transfer completion.

Refer to section 4.11.5.2 for more information
*/
#[repr(C)]
pub struct EventDataTRB {
    /// Event Data Hi and Lo. This field represents the 64-bit value that shall be copied to
    /// the TRB Pointer field (Parameter Component) of the Transfer Event TRB.
    pub data: u64,
    pub status: EventDataTRBStatus,
    pub cmd: EventDataTRBCmd,
}

impl EventDataTRB {
    pub const fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }

    pub const fn new(data: u64, interrupter_target: u16) -> Self {
        Self {
            data,
            status: EventDataTRBStatus::new().with_interrupter_target(interrupter_target),
            cmd: EventDataTRBCmd::new().with_trb_type(TRB_TYPE_EVENT_DATA),
        }
    }
}

#[bitfield(u32)]
pub struct StatusStageTRBStatus {
    #[bits(22)]
    __: (),
    #[bits(10)]
    interrupter_target: u16,
}

#[bitfield(u32)]
pub struct StatusStageTRBCmd {
    #[bits(1)]
    pub cycle_bit: u8,
    /// Evaluate Next TRB (ENT). If this flag is ‘1’ the xHC shall fetch and evaluate the
    /// next TRB before saving the endpoint state. Refer to section 4.12.3 for more information.
    pub ent: bool,
    #[bits(2)]
    __: (),
    /// Chain bit (CH). Set to ‘1’ by software to associate this TRB with the next TRB on the Ring.
    /// A Data Stage TD is defined as a Data Stage TRB followed by zero or more Normal TRBs.
    /// The Chain bit is used to identify a multi-TRB Data Stage TD.
    ///
    /// The Chain bit is always ‘0’ in the last TRB of a Data Stage TD.
    pub chain: bool,
    /// Interrupt On Completion (IOC). If this bit is set to ‘1’, it specifies that when this TRB
    /// completes, the Host Controller shall notify the system of the completion by placing an
    /// Event TRB on the Event ring and sending an interrupt at the next interrupt threshold.
    /// Refer to section 4.10.4.
    pub ioc: bool,
    #[bits(4)]
    __: (),
    #[bits(6)]
    pub trb_type: u8,
    /// Direction (DIR). This bit indicates the direction of the data transfer as defined in the
    /// Data State TRB Direction column of Table 7. If cleared to ‘0’, the data stage transfer
    /// direction is OUT (Write Data).
    ///
    /// If set to ‘1’, the data stage transfer direction is IN (Read Data).
    ///
    /// Refer to section 4.11.2.2 for more information on the use of DIR.
    pub dir_in: bool,
    #[bits(15)]
    __: u16,
}

/**
// xHci Spec Section 6.4.1.2.3 Status Stage TRB Figure 6-11: Status Stage TRB (page 472).

A Status Stage TRB is used to generate the Status stage transaction of a USB
Control transfer. Refer to section 3.2.9 for more information on Control transfers
and the operation of control endpoints.
*/
#[repr(C)]
pub struct StatusStageTRB {
    __rsdv: u64,
    pub status: StatusStageTRBStatus,
    pub cmd: StatusStageTRBCmd,
}

impl StatusStageTRB {
    pub const fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }

    pub const fn new(interrupter_target: u16) -> Self {
        Self {
            __rsdv: 0,
            status: StatusStageTRBStatus::new().with_interrupter_target(interrupter_target),
            cmd: StatusStageTRBCmd::new().with_trb_type(TRB_TYPE_STATUS_STAGE),
        }
    }
}

#[bitfield(u32)]
pub struct NormalTRBStatus {
    #[bits(17)]
    pub trb_transfer_length: u32,
    #[bits(5)]
    pub td_size: u8,
    #[bits(10)]
    pub interrupter_target: u16,
}

#[bitfield(u32)]
pub struct NormalTRBCMD {
    #[bits(1)]
    pub cycle_bit: u8,
    /// Evaluate Next TRB (ENT). If this flag is ‘1’ the xHC shall fetch and evaluate the
    /// next TRB before saving the endpoint state. Refer to section 4.12.3 for more information.
    pub ent: bool,

    /// Interrupt-on Short Packet (ISP). If this flag is ‘1’ and a Short Packet is encountered
    /// for this TRB (i.e., less than the amount specified in TRB Transfer Length), then a
    /// Transfer Event TRB shall be generated with its Completion Code set to Short Packet.
    ///
    /// The TRB Transfer Length field in the Transfer Event TRB shall reflect the residual
    /// number of bytes not transferred into the associated data buffer. In either case, when
    /// a Short Packet is encountered, the TRB shall be retired without error and the xHC shall
    /// advance to the Status Stage TD.
    ///
    /// Note: if the ISP and IOC flags are both ‘1’ and a Short Packet is detected, then only one
    /// Transfer Event TRB shall be queued to the Event Ring. Also refer to section 4.10.1.1.
    pub isp: bool,
    /// No Snoop (NS). When set to ‘1’, the xHC is permitted to set the No Snoop bit in the
    /// Requester Attributes of the PCIe transactions it initiates if the PCIe configuration
    /// Enable No Snoop flag is also set.
    ///
    /// When cleared to ‘0’, the xHC is not permitted to set PCIe packet No Snoop Requester
    /// Attribute. Refer to section 4.18.1 for more information.
    ///
    /// NOTE: If software sets this bit, then it is responsible for maintaining cache consistency.
    pub no_snoop: bool,
    /// Chain bit (CH). Set to ‘1’ by software to associate this TRB with the next TRB on the Ring.
    /// A Data Stage TD is defined as a Data Stage TRB followed by zero or more Normal TRBs.
    /// The Chain bit is used to identify a multi-TRB Data Stage TD.
    ///
    /// The Chain bit is always ‘0’ in the last TRB of a Data Stage TD.
    pub chain: bool,
    /// Interrupt On Completion (IOC). If this bit is set to ‘1’, it specifies that when this TRB
    /// completes, the Host Controller shall notify the system of the completion by placing an
    /// Event TRB on the Event ring and sending an interrupt at the next interrupt threshold.
    /// Refer to section 4.10.4.
    pub ioc: bool,
    /// Immediate Data (IDT). This bit shall be set to ‘1’ in a Setup Stage TRB.
    /// It specifies that the Parameter component of this TRB contains Setup Data.
    pub idt: bool,
    #[bits(3)]
    __: (),
    #[bits(6)]
    pub trb_type: u8,
    /// Direction (DIR). This bit indicates the direction of the data transfer as defined in the
    /// Data State TRB Direction column of Table 7. If cleared to ‘0’, the data stage transfer
    /// direction is OUT (Write Data).
    ///
    /// If set to ‘1’, the data stage transfer direction is IN (Read Data).
    ///
    /// Refer to section 4.11.2.2 for more information on the use of DIR.
    pub dir_in: bool,
    #[bits(15)]
    __: (),
}

#[repr(C)]
pub struct NormalTRB {
    data_buffer_base: PhysAddr,
    pub status: NormalTRBStatus,
    pub cmd: NormalTRBCMD,
}

impl NormalTRB {
    pub const fn into_trb(self) -> TRB {
        unsafe { core::mem::transmute(self) }
    }
    pub const fn new(data_base_addr: PhysAddr, trb_transfer_length: u32, interrupter: u16) -> Self {
        Self {
            data_buffer_base: data_base_addr,
            status: NormalTRBStatus::new()
                .with_interrupter_target(interrupter)
                .with_trb_transfer_length(trb_transfer_length),
            cmd: NormalTRBCMD::new().with_trb_type(TRB_TYPE_NORMAL),
        }
    }
}
