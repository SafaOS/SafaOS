use core::cell::OnceCell;

use bitfield_struct::bitfield;
use safa_abi::net::NicAddrInfoV4;

use crate::{
    PhysAddr, debug,
    drivers::{
        interrupts::{self, IRQInfo, IntTrigger, InterruptReceiver},
        pci::{AllocatedBar, PCICommandReg, PCIDevice},
    },
    error, info,
    memory::{
        frame_allocator::{self, FramePtr},
        paging::{MapToError, PAGE_SIZE},
    },
    net::{
        MacAddress,
        ethernet::{EthernetHeader, EthernetType},
        interface::NetworkInterface,
    },
    process,
    scheduler::wait_queue::WaitQueue,
    sleep, sleep_until,
    thread::{ContextPriority, Tid},
    utils::locks::{Mutex, RwLock},
};

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct RxDescriptor {
    addr: PhysAddr,
    len: u16,
    checksum: u16,
    status: u8,
    error: u8,
    special: u16,
}

impl RxDescriptor {
    pub fn data_mut(&mut self) -> &mut [u8] {
        unsafe {
            &mut (&mut *self.addr.into_virt().into_ptr::<[u8; PAGE_SIZE]>())[..self.len as usize]
        }
    }
}

/// Device Control Register
const REG_CTLR: u16 = 0x0000;
/// Device Status Register
const REG_STATUS: u16 = 0x0008;

/**
 * This register contains the lower bits of the 64-bit descriptor base address.
 * The four low-order, register bits are always ignored. The Receive Descriptor Base Address must point to a 16-byte aligned block of data.
 */
const REG_RDBAL: u16 = 0x2800;
/// This register contains the upper 32 bits of the 64-bit descriptor base address.
const REG_RDBAH: u16 = 0x2804;
/// This register determines the number of bytes allocated to the circular receive descriptor buffer. This
/// value must be 128-byte aligned (the maximum cache line size). Since each descriptor is 16 bytes in
/// length, the total number of receive descriptors is always a multiple of eight.
const REG_RDLEN: u16 = 0x2808;
/// This register contains the head pointer for the receive descriptor buffer. The register points to a 16-
/// byte datum. Hardware controls the pointer. The only time that software should write to this register
/// is after a reset (RCTL.RST or CTRL.RST) and before enabling the receiver function (RCTL.EN).
/// If software were to write to this register while the receive function was enabled, the on-chip
/// descriptor buffers can be invalidated and other indeterminate operations might result. Reading the
/// descriptor head to determine which buffers are finished is not reliable
const REG_RDH: u16 = 0x2810;
/// This register contains the tail pointers for the receive descriptor buffer. The register points to a 16-
/// byte datum. Software writes the tail register to add receive descriptors to the hardware free list for
/// the ring.
const REG_RDT: u16 = 0x2818;
/// This register controls all Ethernet controller receiver functions.
const REG_RCTL: u16 = 0x100;

const _: () = assert!(size_of::<RxDescriptor>() == 16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RctlBSize {
    /// When bsex = 0 means that the size of the buffer is 2048 bytes,
    /// otherwise the size is multiplied by 16 (65536 bytes).
    B2048 = 0b00,
    /// When bsex = 0 means that the size of the buffer is 1024 bytes,
    /// otherwise the size is multiplied by 16 (16384 bytes).
    B1024 = 0b01,
    /// When bsex = 0 means that the size of the buffer is 512 bytes,
    /// otherwise the size is multiplied by 16 (8192 bytes).
    B512 = 0b10,
    /// When bsex = 0 means that the size of the buffer is 256 bytes,
    /// otherwise the size is multiplied by 16 (4096 bytes).
    B256 = 0b11,
}

impl RctlBSize {
    pub const fn from_bits(bits: u8) -> Self {
        match bits {
            0b00 => Self::B2048,
            0b01 => Self::B1024,
            0b10 => Self::B512,
            0b11 => Self::B256,
            _ => unreachable!(),
        }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

/// This register controls all Ethernet controller receiver functions.
#[bitfield(u32)]
struct RegRctl {
    /// Reserved should be set to 0.
    #[bits(1)]
    __rsz0: (),
    /// Receiver Enable
    ///
    /// The receiver is enabled when this bit is 1b. Writing this bit to 0b
    /// stops reception after receipt of any in-progress packets. Data
    /// remains in the receive FIFO until the device is re–enabled.
    /// Disabling or re-enabling the receiver does not reinitialize the packet
    /// filter logic that demarcates packet start and end locations in the
    /// FIFO; Therefore the receiver must be reset before re-enabling it.
    receive_enable: bool,
    /// Store Bad Packets
    ///
    /// 0b = do not store.
    ///
    /// 1b = store bad packets.
    ///
    /// When set, the Ethernet controller stores bad packets (CRC error,
    /// symbol error, sequence error, length error, alignment error, short
    /// packets or where carrier extension or RX_ERR errors) that pass the
    /// filter function in host memory. When the Ethernet controller is in
    /// promiscuous mode, and SBP is set, it might possibly store all
    /// packets
    sbp: bool,
    /// Unicast Promiscuous Enabled
    ///
    /// 0b = Disabled.
    /// 1b = Enabled.
    /// When set, passes without filtering out all received unicast packets.
    /// Otherwise, the Ethernet controller accepts or rejects unicast
    /// packets based on the received packet destination address match
    /// with 1 of the 16 stored addresses.
    upe: bool,
    /// Multicast Promiscuous Enabled
    ///
    /// 0b = Disabled.
    /// 1b = Enabled.
    /// When set, passes without filtering out all received multicast packets.
    /// Otherwise, the Ethernet controller accepts or rejects multicast
    /// packets based on the its 4096-bit vector multicast filtering table.
    mpe: bool,
    /// Long Packet Reception Enable
    ///
    /// 0b = Disabled.
    /// 1b = Enabled.
    /// LPE controls whether long packet reception is permitted. When LPE
    /// is cleared, the Ethernet controller discards packets longer than
    /// 1522 bytes. When LPE is set, the Ethernet controller discards
    /// packets that are longer than 16384 bytes.
    /// For the 82541xx and 82547GI/EI, packets larger than 2 KB require
    /// full duplex operation.
    lpe: bool,
    /// Loopback mode.
    ///
    /// Controls the loopback mode of the Ethernet controller.
    /// 00b = No loopback.
    ///
    /// 01b = Undefined.
    ///
    /// 10b = Undefined.
    ///
    /// 11b = PHY or external SerDes loopback.
    ///
    /// All loopback modes are only allowed when the Ethernet controller is
    /// configured for full-duplex operation. Receive data from transmit
    /// data looped back internally to the SerDes or internal PHY. In TBI
    /// mode (82544GC/EI), the EWRAP signal is asserted.
    /// Note: The 82540EP/EM, 82541xx, and 82547GI/EI do not support
    /// SerDes functionality.
    #[bits(2)]
    lbm: u8,
    /// Receive Descriptor Minimum Threshold Size
    ///
    /// The corresponding interrupt ICR.RXDMT0 is set each time the
    /// fractional number of free descriptors becomes equal to RDMTS.
    /// The following table lists which fractional values correspond to
    /// RDMTS values. The size of the total receiver circular descriptor
    /// buffer is set by [`REG_RDLEN`]. See Section 13.4.27 for details regarding
    /// [`REG_RDLEN`].
    ///
    /// 00b = Free Buffer threshold is set to 1/2 of RDLEN.
    /// 01b = Free Buffer threshold is set to 1/4 of RDLEN.
    /// 10b = Free Buffer threshold is set to 1/8 of RDLEN.
    /// 11b = Reserved.
    #[bits(2)]
    rdmts: u8,
    /// Reserved should be set to zero.
    #[bits(2)]
    __rsz1: (),
    /// Multicast Offset
    ///
    /// The Ethernet controller is capable of filtering multicast packets
    /// based on 4096-bit vector multicast filtering table. The MO
    /// determines which bits of the incoming multicast address are used in
    /// looking up the 4096-bit vector.
    ///
    /// 00b = bits [47:36] of received destination multicast address.
    ///
    /// 01b = bits [46:35] of received destination multicast address.
    ///
    /// 10b = bits [45:34] of received destination multicast address.
    ///
    /// 11b = bits [43:32] of received destination multicast address.
    #[bits(2)]
    mo: u8,
    #[bits(1)]
    /// Reserved should be set to zero, reads as zero always.
    __rsz2: (),
    /// Broadcast Accept Mode.
    ///
    /// 0 = ignore broadcast; 1 = accept broadcast packets.
    ///
    /// When set, passes and does not filter out all received broadcast
    /// packets. Otherwise, the Ethernet controller accepts, or rejects a
    /// broadcast packet only if it matches through perfect or imperfect
    /// filters.
    bam: bool,
    /// Receive Buffer Size
    ///
    /// Controls the size of the receive buffers, allowing the software to
    /// trade off between system performance and storage space. Small
    /// buffers maximize memory efficiency at the cost of multiple
    /// descriptors for bigger packets.
    ///
    /// RCTL.BSEX = 0b:
    ///
    /// - 00b = 2048 Bytes.
    ///
    /// - 01b = 1024 Bytes.
    ///
    /// - 10b = 512 Bytes.
    ///
    /// - 11b = 256 Bytes.
    ///
    /// RCTL.BSEX = 1b:
    ///
    /// - 00b = Reserved; software should not program this value.
    ///
    /// - 01b = 16384 Bytes.
    ///
    /// - 10b = 8192 Bytes.
    ///
    /// - 11b = 4096 Bytes.
    #[bits(2)]
    bsize: RctlBSize,
    /// VLAN Filter Enable1
    ///
    /// 0b = Disabled (filter table does not decide packet acceptance).
    ///
    /// 1b = Enabled (filter table decides packet acceptance for 802.1Q packets).
    ///
    /// Three bits control the VLAN filter table. RCTL.VFE determines
    /// whether the VLAN filter table participates in the packet acceptance
    /// criteria. RCTL.CFIEN and RCTL.CFI are used to decide whether
    /// the CFI bit found in the 802.1Q packet’s tag should be used as part
    /// of the acceptance criteria.
    vfe: bool,

    /// Canonical Form Indicator Enable
    ///
    /// 0b = Disabled (CFI bit found in received 802.1Q packet’s tag is not compared to decide packet acceptance).
    ///
    /// 1b = Enabled (CFI bit found in received 802.1Q packet’s tag must match RCTL.CFI to accept 802.1Q type packet.
    cfifen: bool,
    /// Canonical Form Indicator bit value
    ///
    /// If RCTL.CFIEN is set, then 802.1Q packets with CFI equal to this field is accepted; otherwise, the 802.1Q packet is discarded.
    cfi: bool,
    #[bits(1)]
    /// Reserved should be set to zero.
    __rsz3: (),
    /// Discard Pause Frames
    ///
    /// 0 = incoming pause frames subject to filter comparison.
    ///
    /// 1 = incoming pause frames are filtered out even if they match filter registers.
    ///
    /// DPF controls the DMA function of flow control PAUSE packets
    /// addressed to the station address (RAH/L[0]). If a packet is a valid
    /// flow control packet and is addressed to the station’s address, it is
    /// not transferred to host memory if RCTL.DPF = 1b. However, it is
    /// transferred when DPF is set to 0b.
    dpf: bool,
    /// Pass MAC Control Frames
    ///
    /// 0b = Do not (specially) pass MAC control frames.
    ///
    /// 1b = Pass any MAC control frame (type field value of 8808h) that does not contain the pause opcode of 0001h.
    /// PMCF controls the DMA function of MAC control frames (other than flow control).
    ///
    /// A MAC control frame in this context must be
    /// addressed to either the MAC control frame multicast address or the
    /// station address, match the type field and NOT match the PAUSE
    /// opcode of 0001h. If PMCF = 1b then frames meeting this criteria are
    /// transferred to host memory. Otherwise, they are filtered out.
    pmcf: bool,
    /// Reserved should be set to 0b.
    #[bits(1)]
    __rsz4: (),
    /// Buffer Size Extension
    ///
    /// When set to one, the original BSIZE values are multiplied by 16.
    ///
    /// Refer to the [`RegRCTL::bsize`] bit description.
    bsex: bool,
    /// Strip Ethernet CRC from incoming packet
    ///
    /// 0b = Do not strip CRC field.
    ///
    /// 1b = Strip CRC field.
    ///
    /// Controls whether the hardware strips the Ethernet CRC from the
    /// received packet. This stripping occurs prior to any checksum
    /// calculations. The stripped CRC is not transferred to host memory
    /// and is not included in the length reported in the descriptor.
    secrc: bool,
    #[bits(5)]
    __rsz5: (),
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct TxDescriptor {
    addr: PhysAddr,
    length: u16,
    cso: u8,
    cmd: TxCmd,
    status: u8,
    css: u8,
    special: u16,
}

#[bitfield(u8)]
pub struct TxCmd {
    /// End Of Packet
    ///
    /// When set, indicates the last descriptor making up the packet. One or many
    /// descriptors can be used to form a packet.
    eop: bool,
    /// Insert FCS
    ///
    /// Controls the insertion of the FCS/CRC field in normal Ethernet packets. IFCS is
    /// valid only when EOP is set.
    ifcs: bool,
    /// Insert checksum
    ///
    /// When set, the Ethernet controller needs to insert a checksum at the offset indicated
    /// by the CSO field. The checksum calculations are performed for the entire packet
    /// starting at the byte indicated by the CCS field. IC is ignored if CSO and CCS are out
    /// of the packet range. This occurs when (CSS  length) OR (CSO  length - 1). IC is
    /// valid only when EOP is set.
    ic: bool,
    /// Report Status
    ///
    /// When set, the Ethernet controller needs to report the status information. This ability
    /// may be used by software that does in-memory checks of the transmit descriptors to
    /// determine which ones are done and packets have been buffered in the transmit
    /// FIFO. Software does it by looking at the descriptor status byte and checking the
    /// Descriptor Done (DD) bit.
    rs: bool,
    /// Report Packet Sent
    ///
    /// When set, the 82544GC/EI defers writing the DD bit in the status byte
    /// (DESC.STATUS) until the packet has been sent, or transmission results in an error
    /// such as excessive collisions. It is used is cases where the software must know that
    /// the packet has been sent, and not just loaded to the transmit FIFO. The 82544GC/
    /// EI might continue to prefetch data from descriptors logically after the one with RPS
    /// set, but does not advance the descriptor head pointer or write back any other
    /// descriptor until it sent the packet with the RPS set. RPS is valid only when EOP is
    /// set.
    /// This bit is reserved and should be programmed to 0b for all Ethernet controllers
    /// except the 82544GC/EI.
    rps: bool,
    /// Extension (0b for legacy mode).
    ///
    /// Should be written with 0b for future compatibility.
    ext: bool,
    /// VLAN Packet Enable
    ///
    /// When set, indicates that the packet is a VLAN packet and the Ethernet controller
    /// should add the VLAN Ethertype and an 802.1q VLAN tag to the packet. The
    /// Ethertype field comes from the VET register and the VLAN tag comes from the
    /// special field of the TX descriptor. The hardware inserts the FCS/CRC field in that
    /// case.
    ///
    /// When cleared, the Ethernet controller sends a generic Ethernet packet. The IFCS
    /// controls the insertion of the FCS field in that case.
    /// In order to have this capability CTRL.VME bit should also be set, otherwise VLE
    /// capability is ignored. VLE is valid only when EOP is set.
    vle: bool,
    /// Interrupt Delay Enable
    ide: bool,
}

/// This register contains the lower bits of the 64-bit transmit Descriptor base address. The base
/// register indicates the start of the circular transmit descriptor queue. Since each descriptor is 16 bits
/// in length, the lower four bits are ignored as the Transmit Descriptor Base Address must point to a
/// 16-byte aligned block of data.
const REG_TDBAL: u16 = 0x3800;
/// This register contains the upper 32 bits of the 64-bit transmit Descriptor base address.
const REG_TDBAH: u16 = 0x3804;
/// This register determines the number of bytes allocated to the transmit descriptor circular buffer.
/// This value must be a multiple of 128 bytes (the maximum cache line size). Since each descriptor is
/// 16 bits in length, the total number of receive descriptors is always a multiple of eight.
const REG_TDLEN: u16 = 0x3808;
/// This register contains the head pointer for the transmit descriptor ring. It holds a value that is an
/// offset from the base, and indicates the in–progress descriptor. It points to a 16-byte datum.
/// Hardware controls this pointer. The only time that software should write to this register is after a
/// reset (TCTL.RST or CTRL.RST) and before enabling the transmit function (TCTL.EN). If
/// software were to write to this register while the transmit function was enabled, the on-chip
/// descriptor buffers can be invalidated and indeterminate operation can result. Reading the transmit
/// descriptor head to determine which buffers have been used (and can be returned to the memory
/// pool) is not reliable.
const REG_TDH: u16 = 0x3810;
/// This register contains the tail pointer for the transmit descriptor ring. It holds a value that is an
/// offset from the base, and indicates the location beyond the last descriptor hardware can process.
/// This is the location where software writes the first new descriptor. It points to a 16-byte datum.
/// Software writes the tail pointer to add more descriptors to the transmit ready queue. Hardware
/// attempts to transmit all packets referenced by descriptors between head and tail.
const REG_TDT: u16 = 0x3818;
/// This register controls all transmit functions for the Ethernet controller.
const REG_TCTL: u16 = 0x0400;
/// This register controls the IPG (Inter Packet Gap) timer for the Ethernet controller.
const REG_TIPG: u16 = 0x0410;

#[bitfield(u32)]
struct RegTCTL {
    #[bits(1)]
    __rsz0: (),
    /// Transmit Enable
    ///
    /// The transmitter is enabled when this bit is set to 1b. Writing 0b to
    /// this bit stops transmission after any in progress packets are sent.
    /// Data remains in the transmit FIFO until the device is re-enabled.
    /// Software should combine this operation with reset if the packets in
    /// the TX FIFO should be flushed.
    #[bits(1)]
    en: bool,
    #[bits(1)]
    __rsz1: (),
    /// Pad Short Packets
    ///
    /// 0b = Do not pad.
    ///
    /// 1b = Pad short packets.
    ///
    /// Padding makes the packet 64 bytes long. The padding content is
    /// data.
    /// When the Pad Short Packet feature is disabled, the minimum
    /// packet size the Ethernet controller can transfer to the host is 32
    /// bytes long.
    /// This feature is not the same as Minimum Collision Distance
    /// (TCTL.COLD).
    psp: bool,
    /// Collision Threshold
    ///
    /// This determines the number of attempts at re-transmission prior to
    /// giving up on the packet. The Ethernet back–off algorithm is
    /// implemented and clamps to the maximum value after 16 retries. It
    /// only has meaning in half-duplex operation. Recommended value – 0Fh.
    ct: u8,
    /// Collision Distance
    ///
    /// Specifies the minimum number of byte times that must elapse for
    /// proper CSMA/CD operation. Packets are padded with special
    /// symbols, not valid data bytes. Hardware checks this value and
    /// padded packets even in full-duplex operation.
    ///
    /// Recommended value:
    ///
    /// Half-Duplex – 512-byte time (200h)
    ///
    /// Full-Duplex – 64-byte time (40h)
    ///
    /// Note: 10/100 half-duplex - 64 - 68 (40h to 44h) byte times for the 82541xx and 82547GI/EI only.
    #[bits(10)]
    cold: u16,
    /// Software XOFF Transmission
    ///
    /// When set to 1b, the Ethernet controller schedules the transmission
    /// of an XOFF (PAUSE) frame using the current value of the PAUSE
    /// timer (FCTTV.TTV). This bit self-clears upon transmission of the
    /// XOFF frame. This bit is valid only in Full-Duplex mode of
    /// operation. Software should not set this bit while the Ethernet
    /// controller is configured for half-duplex operation.
    swxoff: bool,
    #[bits(1)]
    __rsz2: (),
    /// Re-transmit on Late Collision
    ///
    /// When set, enables the Ethernet controller to re-transmit on a late
    /// collision event.
    ///
    /// The collision window is speed dependent. For example, 64 bytes
    /// for 10/100 Mb/s and 512 bytes for 1000Mb/s operation. If a late
    /// collision is detected when this bit is disabled, the transmit function
    /// assumes the packet is successfully transmitted.
    ///
    /// The RTLC bit is ignored in full-duplex mode.
    rtlc: bool,
    #[bits(7)]
    __rsz3: (),
}

/// Link speed encoded in CTRL.SPEED (2 bits)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LinkSpeed {
    /// 10 Mb/s
    S10 = 0,
    /// 100 Mb/s
    S100 = 1,
    /// 1000 Mb/s
    S1000 = 2,
    /// Reserved / unknown value (0b11)
    Reserved = 3,
}

impl LinkSpeed {
    /// Create a `LinkSpeed` from the low 2 bits of `bits`.
    ///
    /// This is `const` so it can be used by bitfield crates that require a
    /// compile-time constructor. It is *lossy* — unknown values are clamped
    /// into `Reserved` by masking the input to 2 bits.
    pub const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => LinkSpeed::S10,
            1 => LinkSpeed::S100,
            2 => LinkSpeed::S1000,
            _ => LinkSpeed::Reserved,
        }
    }

    /// Return the raw bits for embedding in a bitfield.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    pub const fn speed_mb(self) -> u32 {
        match self {
            LinkSpeed::S10 => 10,
            LinkSpeed::S100 => 100,
            LinkSpeed::S1000 => 1000,
            LinkSpeed::Reserved => 0,
        }
    }
}

#[bitfield(u32)]
pub struct RegCTRL {
    /// Full Duplex
    ///
    /// 1b = Full duplex enabled. This bit can be overridden by auto-negotiation
    /// or PHY/SerDes logic in some modes.
    #[bits(1)]
    fd: bool,
    #[bits(2)]
    __rsz0: (),
    /// Link Reset
    ///
    /// Writing 1b forces a link-reset (used with internal SerDes/TBI). LRST=1
    /// disables auto-negotiation; LRST->0 restarts it.
    #[bits(1)]
    lrst: bool,
    #[bits(1)]
    __rsz1: (),
    /// Auto-Speed Detection Enable
    ///
    /// Enable hardware auto-detection of link speed (ASD).
    #[bits(1)]
    asde: bool,
    /// Set Link Up (force link)
    ///
    /// Forces the MAC link up when set (ignored if auto-negotiation is enabled).
    #[bits(1)]
    slu: bool,
    /// Invert Loss-of-Signal (LOS)
    ///
    /// Inverts the LOS polarity (useful for some external PHY/TBI setups).
    #[bits(1)]
    ilos: bool,
    /// Speed select
    ///
    /// 00 = 10 Mb/s, 01 = 100 Mb/s, 10 = 1000 Mb/s, 11 = reserved.
    #[bits(2)]
    speed: LinkSpeed,
    #[bits(1)]
    __rsz2: (),

    /// Force Speed
    ///
    /// When set, use CTRL.SPEED rather than auto-detection (can be superseded
    /// by CTRL_EXT.SPD_BYPS).
    #[bits(1)]
    frcspd: bool,
    /// Force Duplex
    ///
    /// When set, software controls duplex via CTRL.FD; otherwise duplex comes
    /// from PHY/negotiation.
    #[bits(1)]
    frcdplx: bool,
    /// EEPROM Reset (self-clearing)
    ///
    /// Triggers an EEPROM "reset-like" read sequence; this bit is self-clearing.
    #[bits(1)]
    ee_rst: bool,
    #[bits(1)]
    __rsz3: (),
    /// Speed Select Bypass
    ///
    /// When set, bypass auto-speed and immediately use CTRL.SPEED.
    #[bits(1)]
    spd_byps: bool,
    #[bits(1)]
    __rsz4: (),
    /// Relaxed Ordering Disable (RO-DIS)
    ///
    /// Controls relaxed ordering in PCI-X mode (chip-variant dependent).
    #[bits(1)]
    ro_dis: bool,
    /// SDP0 Data (software-definable pin)
    ///
    /// Used to read/write SDP0 pin value when configured as GPIO.
    #[bits(1)]
    sdp0_data: bool,
    /// SDP1 Data (software-definable pin)
    ///
    /// Used to read/write SDP1 pin value when configured as GPIO.
    #[bits(1)]
    sdp1_data: bool,
    /// Advertise D3Cold Wakeup Capability Enable
    ///
    /// Controls whether D3Cold wakeup capability is advertised.
    #[bits(1)]
    advd3wuc: bool,
    /// PHY Power-Management Enable
    ///
    /// When set, PHY is informed of power-state transitions and may
    /// auto-negotiate lower speeds in D3/D0u for manageability/wakeup.
    #[bits(1)]
    en_phy_pwr_mgmt: bool,
    /// SDP0 direction (0=input, 1=output)
    #[bits(1)]
    sdp0_iodir: bool,
    /// SDP1 direction (0=input, 1=output)
    #[bits(1)]
    sdp1_iodir: bool,
    #[bits(2)]
    __rsz5: (),
    /// Device Reset (self-clearing)
    ///
    /// Writing 1b performs a global device reset (except PCI config space).
    #[bits(1)]
    rst: bool,
    /// Receive Flow Control Enable
    ///
    /// When set, the device responds to incoming flow-control PAUSE frames.
    #[bits(1)]
    rfce: bool,
    /// Transmit Flow Control Enable
    ///
    /// When set, the device may transmit XON/XOFF (PAUSE) frames.
    #[bits(1)]
    tfce: bool,
    #[bits(1)]
    __rsz6: (),
    /// VLAN Mode Enable
    ///
    /// When set, transmit descriptors with VLE will include 802.1Q headers.
    #[bits(1)]
    vme: bool,
    /// PHY Reset
    ///
    /// Assert/reset for internal PHY. Write 1, delay ~3µs, clear bit to release.
    #[bits(1)]
    phy_rst: bool,
}

/// Device Status Register (STATUS) bitfield
///
/// Based on Table 13-5 (Device Status Register) from the Intel e1000 manual.
/// This maps the low 8 bits used by software; upper bits are reserved.
///
/// Note: this is the read-only STATUS register; treat writes as no-op.
#[bitfield(u32)]
#[derive(PartialEq, Eq)]
pub struct RegSTATUS {
    /// Full Duplex indication. Reflects CTRL.FD or negotiated duplex.
    #[bits(1)]
    fd: bool,
    /// Link Up indication. 1 = link up (negotiated or forced).
    #[bits(1)]
    lu: bool,
    /// Function ID (LAN identifier). For most devices this is 0b.
    /// 2-bit field: [0b,0b] LAN A, [0b,1b] LAN B (82546GB/EB only).
    #[bits(2)]
    function_id: u8,
    /// Transmit Paused — set when transmitter is paused due to received XOFF.
    #[bits(1)]
    txoff: bool,
    /// TBI Mode / Internal SerDes indication.
    /// 1 = TBI or internal SerDes; 0 = internal PHY mode.
    #[bits(1)]
    tbimode: bool,
    /// Speed status bits (2 bits).
    /// 00 = 10Mb/s, 01 = 100Mb/s, 10 = 1000Mb/s, 11 = reserved/unknown.
    #[bits(2)]
    speed: LinkSpeed,
    /// Reserved upper bits (8..31)
    #[bits(24)]
    __rsz0: (),
}

const _: () = assert!(size_of::<TxDescriptor>() == 16);

const RX_DESC_COUNT: usize = 256 /* 1 pages */;
const TX_DESC_COUNT: usize = 256 /* 1 pages */;

/// Interrupt Mask Clear Register
///
/// Software uses this register to disable an interrupt. Interrupts are presented to the bus interface only
///
/// when the mask bit is set to 1b and the cause bit set to 1b. The status of the mask bit is reflected in
/// the Interrupt Mask Set/Read Register (see Section 13.4.20), and the status of the cause bit is
/// reflected in the Interrupt Cause Read Register (see Section 13.4.17).
/// Software blocks interrupts by clearing the corresponding mask bit. This is accomplished by writing
/// a 1b to the corresponding bit in this register. Bits written with 0b are unchanged (their mask status
/// does not change).
const REG_IMC: u16 = 0xD8;
/// Interrupt Mask Set/Read Register
const REG_IMS: u16 = 0xD0;
/// Receiver Timer Interrupt mask
///
/// Set when the receiver timer expires.
/// The receiver timer is used for receiver descriptor packing. Timer
/// expiration flushes any accumulated descriptors and sets an
/// interrupt event when enabled.
const IMS_RXT0_MASK: u32 = 1 << 7;
/// No idea where this is documented but everyone does it...
const IMS_RXQ0_MASK: u32 = 1 << 20;
/// Receiver Timer Interrupt Cause
///
/// Set when the receiver timer expires.
/// The receiver timer is used for receiver descriptor packing. Timer
/// expiration flushes any accumulated descriptors and sets an
/// interrupt event when enabled.
const ICR_RXT0: u32 = IMS_RXT0_MASK;
/// No idea where this is documented but everyone does it...
const ICR_RXQ0: u32 = IMS_RXQ0_MASK;
/// Interrupt Cause Read Register
///
/// This register contains all interrupt conditions for the Ethernet controller. Each time an interrupt
/// causing event occurs, the corresponding interrupt bit is set in this register. A PCI interrupt is
/// generated each time one of the bits in this register is set, and the corresponding interrupt is enabled
/// through the Interrupt Mask Set/Read IMS Register
///
/// All register bits are cleared upon read. As a result, reading this register implicitly acknowledges
/// any pending interrupt events. Writing a 1b to any bit in the register also clears that bit. Writing a 0b
/// to any bit has no effect on that bit.
const REG_ICR: u16 = 0xC0;
/// Interrupt Throttling Register
///
/// This register controls the minimum inter-interrupt interval. The interval is specified in 256 ns
/// increments. Setting this bit to 0b disables interrupt throttling logic.
const REG_ITR: u16 = 0xC4;
/// Receive Delay Timer Register
/// This register is used to delay interrupt notification for the receive descriptor ring. Delaying
/// interrupt notification helps maximize the number of receive packets serviced by a single interrupt.
const REG_RDTR: u16 = 0x2820;

const INTERRUPT_THROTTLING_RATE: u32 = 500;

#[derive(Debug, Clone)]
pub struct E1000Comm {
    receive_descriptors: FramePtr<[RxDescriptor; RX_DESC_COUNT]>,
    transmit_descriptors: FramePtr<[TxDescriptor; TX_DESC_COUNT]>,
    tx_curr: u32,
}

impl E1000Comm {
    pub const fn new(
        receive_descriptors: FramePtr<[RxDescriptor; RX_DESC_COUNT]>,
        transmit_descriptors: FramePtr<[TxDescriptor; TX_DESC_COUNT]>,
    ) -> Self {
        Self {
            receive_descriptors,
            transmit_descriptors,
            tx_curr: 0,
        }
    }

    pub fn send_ethernet(
        &mut self,
        card: &E1000NetCard,
        dest_mac: MacAddress,
        ethertype: EthernetType,
        payload: &[u8],
    ) -> Result<(), ()> {
        if payload.is_empty() {
            return Ok(());
        }

        if payload.len() > PAGE_SIZE - size_of::<EthernetHeader>() {
            return Err(());
        }

        let src_mac = card.mac_address();
        let header = EthernetHeader::new(dest_mac, src_mac, ethertype);

        let tx_descs = &mut *self.transmit_descriptors;

        let desc_count = tx_descs.len();
        let curr_tx = &mut tx_descs[self.tx_curr as usize];
        let tx_virt = curr_tx.addr.into_virt();

        unsafe {
            let ptr = tx_virt.into_ptr::<[u8; PAGE_SIZE]>();
            let header_size = size_of::<EthernetHeader>();

            let header_ptr = ptr as *mut EthernetHeader;
            let payload_ptr = ptr.byte_add(header_size) as *mut u8;

            header_ptr.write_volatile(header);
            payload_ptr.copy_from(payload.as_ptr(), payload.len());

            core::ptr::write_volatile(
                curr_tx,
                TxDescriptor {
                    addr: curr_tx.addr,
                    length: (header_size + payload.len()) as u16,
                    cso: 0,
                    cmd: TxCmd::new()
                        .with_eop(true)
                        .with_ifcs(true)
                        .with_rs(true)
                        .with_rps(true),
                    status: 0,
                    css: 0,
                    special: 0,
                },
            );
        }

        self.tx_curr += 1;
        if self.tx_curr as usize >= desc_count {
            self.tx_curr = 0;
        }

        card.write_command(REG_TDT, self.tx_curr);
        while crate::drivers::utils::read_ref!(curr_tx.status) == 0 {
            core::hint::spin_loop();
        }
        card.clear_status();

        debug!(
            E1000NetCard,
            "Transmitted packet worth {} bytes, header: {:#?}, descriptor is: {:#x?}",
            payload.len(),
            header,
            curr_tx
        );
        Ok(())
    }
}

#[derive(Debug)]
pub struct E1000NetCard {
    base: AllocatedBar,
    eeprom_exists: OnceCell<bool>,
    mac: OnceCell<MacAddress>,
    com: OnceCell<Mutex<E1000Comm>>,
    irq_info: IRQInfo,
    addr_info: RwLock<NicAddrInfoV4>,
    wait_queue: Mutex<WaitQueue<1>>,
}

impl E1000NetCard {
    pub fn write_command(&self, p_addr: u16, p_value: u32) {
        unsafe { self.base.write_u32(p_addr, p_value) }
    }

    pub fn read_command(&self, p_addr: u16) -> u32 {
        unsafe { self.base.read_u32(p_addr) }
    }

    pub fn write_reg_ctrl(&self, v: RegCTRL) {
        self.write_command(REG_CTLR, v.into_bits());
    }

    pub fn read_reg_ctrl(&self) -> RegCTRL {
        let val = self.read_command(REG_CTLR);
        RegCTRL::from_bits(val)
    }

    pub fn status(&self) -> RegSTATUS {
        let val = self.read_command(REG_STATUS);
        RegSTATUS::from_bits(val)
    }

    pub fn clear_status(&self) {
        _ = self.status()
    }

    const REG_EEPROM: u16 = 0x14;

    pub fn eeprom_exists(&self) -> bool {
        if let Some(eeprom_exists) = self.eeprom_exists.get() {
            return *eeprom_exists;
        } else {
            core::hint::cold_path();

            let mut eeprom_exists: bool = false;
            self.write_command(Self::REG_EEPROM, 1);
            // tries to wait for eeprom to respond
            for _ in 0..1500 {
                let val = self.read_command(Self::REG_EEPROM);
                if (val & 0x10) != 0 {
                    eeprom_exists = true;
                    break;
                }
            }
            _ = self.eeprom_exists.set(eeprom_exists);
            eeprom_exists
        }
    }

    /// Perform a u16 read from the EEPROM with the addr `addr` if it exists, panicks if it doesn't.
    pub fn eeprom_read(&self, addr: u8) -> u16 {
        assert!(
            self.eeprom_exists.get().is_some_and(|e| *e),
            "Must ensure EEPROM exists before performing an EEPROM read"
        );

        self.write_command(Self::REG_EEPROM, ((addr as u32) << 8) | 1);

        let val = sleep_until!(1000 ms, let tmp = self.read_command(Self::REG_EEPROM); until (tmp & 0x10) != 0).expect("Timeout waiting for an eeprom read");
        (val >> 16) as u16
    }

    const RXADDR_LO: u16 = 0x5400;
    const RXADDR_HI: u16 = 0x5404;

    /// Retrieves and caches the MAC Address
    pub fn mac_address(&self) -> MacAddress {
        if let Some(mac) = self.mac.get() {
            *mac
        } else {
            let mut mac = [0u8; 6];
            if self.eeprom_exists() {
                for addr in 0..3 {
                    let read = self.eeprom_read(addr);
                    let i = addr as usize * 2;
                    mac[i] = read as u8;
                    mac[i + 1] = (read >> 8) as u8;
                }
            } else {
                let mac_low = self.read_command(Self::RXADDR_LO);
                let mac_hi = self.read_command(Self::RXADDR_HI);

                mac[..4].copy_from_slice(&mac_low.to_le_bytes());
                mac[4..].copy_from_slice(&mac_hi.to_le_bytes()[..2]);
            }

            _ = self.mac.set(MacAddress::new(mac));
            MacAddress::new(mac)
        }
    }

    pub fn try_rx_init(&self) -> Result<FramePtr<[RxDescriptor; RX_DESC_COUNT]>, MapToError> {
        // TODO: proper VMM or DMA allocation at least
        const RX_DESCS_BYTES: usize = RX_DESC_COUNT * size_of::<RxDescriptor>();
        const {
            assert!(RX_DESCS_BYTES <= PAGE_SIZE);
        }

        let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

        let mut descriptors_ref = unsafe { frame.into_ptr::<[RxDescriptor; RX_DESC_COUNT]>() };
        let descriptors_phys = descriptors_ref.phys_addr().into_raw();

        // Initializes each descriptor
        for descriptor in descriptors_ref.iter_mut() {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
            unsafe {
                frame.into_ptr::<[u8; PAGE_SIZE]>().fill(0);
                *descriptor = core::mem::zeroed();
            }

            descriptor.addr = frame.phys_addr();
        }

        // Address setup
        self.write_command(REG_RDBAL, descriptors_phys as u32);
        self.write_command(REG_RDBAH, (descriptors_phys >> 32) as u32);
        // Length setup
        self.write_command(REG_RDLEN, RX_DESCS_BYTES as u32);
        // Head/Tail setup
        self.write_command(REG_RDH, 0);
        self.write_command(REG_RDT, RX_DESC_COUNT as u32 - 1);

        // Control setup
        self.write_command(
            REG_RCTL,
            RegRctl::new()
                .with_receive_enable(true)
                .with_sbp(true)
                .with_upe(true)
                .with_mpe(true)
                .with_bam(true)
                .with_secrc(true)
                .with_bsex(true) // :3
                .with_bsize(RctlBSize::B256) // bsex is 1
                .into_bits(),
        );
        self.clear_status();

        Ok(descriptors_ref)
    }

    pub fn try_tx_init(&self) -> Result<FramePtr<[TxDescriptor; TX_DESC_COUNT]>, MapToError> {
        const TX_DESCS_BYTES: usize = TX_DESC_COUNT * size_of::<TxDescriptor>();
        const {
            assert!(TX_DESCS_BYTES <= PAGE_SIZE);
        }

        // TODO: proper DMA
        let frame = frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;

        let mut descriptors_ref = unsafe { frame.into_ptr::<[TxDescriptor; TX_DESC_COUNT]>() };
        let descriptors_phys = descriptors_ref.phys_addr().into_raw();
        // Initializes each descriptor
        for descriptor in descriptors_ref.iter_mut() {
            let frame =
                frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?;
            unsafe {
                frame.into_ptr::<[u8; PAGE_SIZE]>().fill(0);
                *descriptor = core::mem::zeroed();
            }

            descriptor.addr = frame.phys_addr();
            descriptor.cmd = TxCmd::new().with_eop(true);
        }

        // Descriptor base address setup
        self.write_command(REG_TDBAL, descriptors_phys as u32);
        self.write_command(REG_TDBAH, (descriptors_phys >> 32) as u32);
        // Length setup
        self.write_command(REG_TDLEN, TX_DESCS_BYTES as u32);
        // Tail/Head setup (no transmissions)
        self.write_command(REG_TDH, 0);
        self.write_command(REG_TDT, 0);

        // Setup ctl register
        let reg_tctl = RegTCTL::from_bits(self.read_command(REG_TCTL));
        self.write_command(
            REG_TCTL,
            reg_tctl
                .with_en(true)
                .with_psp(true)
                .with_rtlc(true)
                .with_ct(0x10)
                .into_bits(),
        );

        // No idea why everyone does it this way and I am too lazy to read and wriite docs...`
        self.write_command(REG_TIPG, 0x60200a);

        Ok(descriptors_ref)
    }

    /// Init the packet receive and transfer process (TX and RX).
    pub fn comm_init(&self) -> Result<(), MapToError> {
        let tx_descs = self.try_tx_init()?;
        let rx_descs = self.try_rx_init()?;

        self.com
            .set(Mutex::new(E1000Comm::new(rx_descs, tx_descs)))
            .expect("Already initialized");
        Ok(())
    }

    pub fn enable_link(&self) {
        self.write_reg_ctrl(self.read_reg_ctrl().with_slu(true));
    }

    pub fn get_link_speed(&self) -> LinkSpeed {
        let status = self.status();
        status.speed()
    }

    pub fn disable_interrupts(&self) {
        self.write_command(REG_IMC, u32::MAX);
        self.write_command(REG_ICR, u32::MAX);
        self.clear_status();
    }

    pub fn enable_interrupts(&self) {
        self.write_command(REG_RDTR, 0);
        self.write_command(REG_ITR, INTERRUPT_THROTTLING_RATE);
        self.write_command(REG_IMS, IMS_RXQ0_MASK | IMS_RXT0_MASK);
        self.write_command(REG_ICR, u32::MAX);
        self.clear_status();
    }

    pub fn reset(&self) {
        self.disable_interrupts();
        sleep!(100 ms);

        self.write_command(REG_RCTL, 0);
        self.write_command(REG_TCTL, RegTCTL::new().with_psp(true).into_bits());
        self.clear_status();

        let rctlr = self.read_reg_ctrl();
        self.write_reg_ctrl(rctlr.with_rst(true));
        sleep!(500 ms);

        self.disable_interrupts();
    }

    pub fn init(&self) -> Result<(), MapToError> {
        self.reset();
        self.enable_link();

        // Clear the statistical counters
        for i in 0..128 {
            self.write_command(0x5200 + i * 4, 0);
        }

        for i in 0..64 {
            _ = self.read_command(0x4000 + i * 4);
        }

        let mac = self.mac_address();
        info!(E1000NetCard, "Initializing with mac {}", mac);

        self.comm_init()?;
        self.enable_interrupts();

        let link_speed = self.get_link_speed();
        info!(
            E1000NetCard,
            "Link speed is {} MB/s, link up: {}",
            link_speed.speed_mb(),
            self.status().lu()
        );
        Ok(())
    }

    pub fn send_ethernet(
        &self,
        dest_mac: MacAddress,
        ethertype: EthernetType,
        payload: &[u8],
    ) -> Result<(), ()> {
        self.com
            .get()
            .expect("E1000 Driver not initialized")
            .lock()
            .send_ethernet(self, dest_mac, ethertype, payload)
    }
}

unsafe impl Sync for E1000NetCard {}
impl NetworkInterface for E1000NetCard {
    fn name(&self) -> &'static str {
        "E1000"
    }

    fn mac_address(&self) -> MacAddress {
        self.mac_address()
    }

    fn send_ethernet(
        &self,
        dst_mac: MacAddress,
        ethertype: EthernetType,
        payload: &[u8],
    ) -> Result<(), crate::net::interface::NetIntError> {
        self.send_ethernet(dst_mac, ethertype, payload)
            .map_err(|()| crate::net::interface::NetIntError::PacketTooLarge)
    }

    fn nic_info(&self) -> NicAddrInfoV4 {
        *self.addr_info.read()
    }

    fn set_nic_info(&self, info: NicAddrInfoV4) {
        *self.addr_info.write() = info;
    }

    fn ipv4_address(&self) -> core::net::Ipv4Addr {
        self.addr_info.read().ipv4_address
    }
}

fn e1000_poll_thread(tid: Tid, nic: &'static E1000NetCard) -> ! {
    info!(E1000NetCard, "Polling on thread {tid}");

    loop {
        let mut comm = nic.com.get().expect("E1000 Driver not initialized").lock();
        // NOTE: Won't deadlock if an E1000 interrupt happens because of `Mutex`.
        let pending_wait = nic.wait_queue.prepare_wait();
        let curr = (nic.read_command(REG_RDT) + 1) as usize % RX_DESC_COUNT;
        let desc = &mut comm.receive_descriptors[curr];

        if (desc.status & 1) == 0 {
            drop(comm);
            pending_wait.enter_wait((), None).expect("Failed to wait");
            continue;
        }

        if desc.error != 0 {
            error!(E1000NetCard, "Received packet error: {:#x}", desc.error);
        }

        let bytes = desc.data_mut();
        crate::net::handle_packet(nic, bytes);

        crate::write_ref!(desc.status, 0);
        nic.write_command(REG_RDT, curr as u32);
    }
}

impl InterruptReceiver for E1000NetCard {
    fn handle_interrupt(&'static self) -> bool {
        let icr = self.read_command(REG_ICR);
        if icr == 0 {
            return false;
        }

        // Before we read any registers our anything,
        // We have to lock the polling thread from reading registers, before polling any registers the polling thread must also lock the wait queue.
        // TODO: This could be represented better by putting requiring a combined lock on write_command, read_command
        let mut wait_queue = self.wait_queue.lock();
        debug!(E1000NetCard, "Interrupt received: {icr:#x}");
        self.write_command(REG_ICR, icr);

        if (icr & (ICR_RXQ0 | ICR_RXT0)) != 0 {
            wait_queue.wake_all();
        }

        debug!(
            E1000NetCard,
            "Handled interrupt with status: {:?}",
            self.status()
        );
        true
    }
}

impl PCIDevice for E1000NetCard {
    const CLASS_SUBCLASS: (u8, u8) = (0x2, 0x0);
    const VENDOR_ID: Option<&[u16]> = Some(&[0x8086]);
    const DEVICE_ID: Option<&[u16]> = Some(&[
        0x100E, /* VMS */
        0x153A, /* Intel I217 */
        0x10EA, /* Intel 82577LM */
    ]);
    fn create(mut info: crate::drivers::pci::PCIDeviceInfo) -> Result<Self, &'static str>
    where
        Self: Sized,
    {
        debug!(E1000NetCard, "Creating: {info:#?}");
        let bars = info.get_bars();
        let irq_info = info
            .get_best_irq_info(&[] /* We currently only allocate the base BAR */)
            .expect("E1000 must support interrupts");
        let general_header = info.unwrap_general();

        let (allocated_bars, _, _) = AllocatedBar::allocate_bars::<6>(&"E1000", &*bars);
        let base_bar = allocated_bars[0];
        match base_bar {
            AllocatedBar::Memory(base_virt, _) => {
                info!(E1000NetCard, "Mapped starting at {base_virt:?} ");
                general_header
                    .common()
                    .write_command(PCICommandReg::BUS_MASTER | PCICommandReg::MEM_SPACE);
            }
            AllocatedBar::IO(port, size) => {
                info!(E1000NetCard, "Using IO with base {port} and size {size}");
                general_header
                    .common()
                    .write_command(PCICommandReg::BUS_MASTER | PCICommandReg::IO_SPACE);
            }
        }

        // FIXME: More errors.
        Ok(E1000NetCard {
            irq_info,
            base: base_bar,
            mac: OnceCell::new(),
            eeprom_exists: OnceCell::new(),
            com: OnceCell::new(),
            addr_info: RwLock::new(NicAddrInfoV4::default()),
            wait_queue: Mutex::new(WaitQueue::new()),
        })
    }

    fn start(&'static self) -> bool {
        interrupts::register_irq(self.irq_info.clone(), IntTrigger::LevelDeassert, self);

        if let Err(err) = self.init() {
            error!(E1000NetCard, "Init failed with err: {:?}", err);
            return false;
        }

        process::current::kernel_thread_spawn(
            e1000_poll_thread,
            self,
            Some(ContextPriority::High),
            None,
        )
        .expect("Failed to spawn E1000 poll thread");
        crate::net::add_interface(self);
        true
    }
}
