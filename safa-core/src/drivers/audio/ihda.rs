use core::{cell::SyncUnsafeCell, fmt::Debug, num::NonZero, sync::atomic::AtomicU16};

use alloc::{boxed::Box, collections::vec_deque::VecDeque, sync::Arc, vec::Vec};
use bitfield_struct::bitfield;

use crate::{
    PhysAddr,
    arch::without_interrupts,
    audio::{
        self,
        interface::{AudioCard, AudioInfo},
    },
    debug,
    dma::DMABuffer,
    drivers::{
        audio::ihda::{self},
        interrupts::{self, IRQInfo, InterruptReceiver},
        pci::{AllocatedBar, PCICommandReg, PCIDevice},
        vfs::FSResult,
    },
    error, info,
    memory::{AlignTo, paging::PAGE_SIZE},
    scheduler::wait_queue::{WaitError, WaitQueue, WaitQueueWithTimeout},
    thread::Tid,
    utils::locks::SpinLock,
    warn, write_ref,
};

mod param {
    pub const SUB_NODE_COUNT: u8 = 0x04;
    pub const FUNCTION_GROUP_TYPE: u8 = 0x05;
    pub const AUDIO_WIDGET_CAPS: u8 = 0x09;
    pub const PIN_CAPS: u8 = 0x0C;
    pub const CONNECTION_LIST_LEN: u8 = 0x0E;
    pub const SUPPORTED_PCM_SIZE_RATES: u8 = 0xA;
    pub const AMP_OUT: u8 = 0x12;
}

const AMP_SET_LEFT: u16 = 1 << 13;
const AMP_SET_RIGHT: u16 = 1 << 12;
const AMP_SET_OUTPUT: u16 = 1 << 15;
const AUDIO_FUNCTION_GROUP: u8 = 0x1;
const VERB_GET_PARAMETER: u16 = 0xF00;
const VERB_GET_CONFIG_DEFAULT: u16 = 0xF1C;
const VERB_GET_CONNECTION_LIST_ENTRY: u16 = 0xF02;

const VERB_SET_AMP_GAIN_MUTE: u8 = 0x3;
const VERB_SET_CONVERTOR_CHANNEL: u16 = 0x706;
const VERB_SET_CONVERTOR_FMT: u8 = 0x2;
const VERB_SET_EAPD_BTL_ENB: u16 = 0x70C;
const VERB_SET_POWER_STATE: u16 = 0x705;
const VERB_SET_PIN_CTL: u16 = 0x707;
const VERB_SET_CONNECTION_SELECTOR: u16 = 0x701;

#[derive(Debug, Clone, Copy)]
pub enum DefaultDevice {
    LineOut = 0x0,
    Speaker = 0x1,
    HPOut = 0x2,
    CD = 0x3,
    SPDIFOut = 0x4,
    DigitalOtherOut = 0x5,
    ModemLineSide = 0x6,
    ModemHeadsetSide = 0x7,
    LineIn = 0x8,
    AUX = 0x9,
    MicIn = 0xA,
    Telephony = 0xB,
    SPDIFIn = 0xC,
    DigitalOtherIn = 0xD,
    Reserved = 0xE,
    Other = 0xF,
}

impl DefaultDevice {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0xF {
            0x0 => Self::LineOut,
            0x1 => Self::Speaker,
            0x2 => Self::HPOut,
            0x3 => Self::CD,
            0x4 => Self::SPDIFOut,
            0x5 => Self::DigitalOtherOut,
            0x6 => Self::ModemLineSide,
            0x7 => Self::ModemHeadsetSide,
            0x8 => Self::LineIn,
            0x9 => Self::AUX,
            0xA => Self::MicIn,
            0xB => Self::Telephony,
            0xC => Self::SPDIFIn,
            0xD => Self::DigitalOtherIn,
            0xE => Self::Reserved,
            0xF => Self::Other,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortConnectivity {
    Jack = 0b00,
    None = 0b01,
    Fixed = 0b10,
    FixedAndJack = 0b11,
}

impl PortConnectivity {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0b00 => Self::Jack,
            0b01 => Self::None,
            0b10 => Self::Fixed,
            0b11 => Self::FixedAndJack,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PinComplex {
    caps: u32,
    config_default: u32,
}

impl Debug for PinComplex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PinComplex")
            .field("default_device", &self.default_device())
            .field("port_connectivity", &self.port_connectivity())
            .field("sequence", &self.sequence())
            .field("association", &self.association())
            .field("is_out", &self.is_output_capable())
            .finish()
    }
}

impl PinComplex {
    #[inline]
    pub const fn default_device(&self) -> DefaultDevice {
        DefaultDevice::from_bits(((self.config_default >> 20) & 0xF) as u8)
    }

    #[inline]
    pub const fn port_connectivity(&self) -> PortConnectivity {
        PortConnectivity::from_bits(((self.config_default >> 30) & 0x3) as u8)
    }

    #[inline]
    pub const fn association(&self) -> u8 {
        ((self.config_default >> 4) & 0xF) as u8
    }
    #[inline]
    pub const fn sequence(&self) -> u8 {
        (self.config_default & 0xF) as u8
    }

    #[inline]
    pub const fn is_output_capable(&self) -> bool {
        (self.caps & (1 << 4)) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    AudioOutput,
    AudioInput,
    Mixer,
    Selector,
    PinComplex(PinComplex),
    PowerWidget,
    VolumeKnob,
    BeepGenerator,
    Unknown(u8),
}

#[derive(Debug, Clone)]
pub struct Widget {
    node_id: u8,
    kind: WidgetKind,
    raw_caps: u32,
    connections: Vec<u16>,
}

impl Widget {
    pub const fn has_out_amp(&self) -> bool {
        (self.raw_caps & (1 << 2)) != 0
    }
    pub const fn supports_stereo(&self) -> bool {
        (self.raw_caps & 1) != 0
    }

    pub fn discover(hda: &IntelHDA, codec: u8, node: u8) -> Result<Self, WaitError> {
        let raw_caps = hda.get_param(codec, node, param::AUDIO_WIDGET_CAPS)?;
        debug!(ihda::Widget, "Caps: {raw_caps}, node: {node}");
        let raw_ty = ((raw_caps >> 20) & 0x0F) as u8;

        let kind = match raw_ty {
            0 => WidgetKind::AudioOutput,
            1 => WidgetKind::AudioInput,
            2 => WidgetKind::Mixer,
            3 => WidgetKind::Selector,
            4 => {
                let pin_caps = hda.get_param(codec, node, param::PIN_CAPS)?;
                let config_default = hda.send_command(
                    HDACommand::new()
                        .with_codec_addr(codec)
                        .with_node_idx(node)
                        .with_command(VERB_GET_CONFIG_DEFAULT)
                        .with_data(0),
                )?;
                WidgetKind::PinComplex(PinComplex {
                    caps: pin_caps,
                    config_default,
                })
            }
            5 => WidgetKind::PowerWidget,
            6 => WidgetKind::VolumeKnob,
            7 => WidgetKind::BeepGenerator,
            other => WidgetKind::Unknown(other),
        };

        let mut conn_len = hda.get_param(codec, node, param::CONNECTION_LIST_LEN)?;
        let is_long = (conn_len & 0x80) != 0;
        conn_len &= 0x7F;

        let mut connections = Vec::new();

        let per_resp = if is_long { 2 } else { 4 };
        let mut c = 0;
        // FIXME: This function was translated by claude from Ethereal's iHDA driver
        // In general connections are 8-bit long but sometimes they are 16-bits, connections come toghether packed in a single u32
        // We may not be handling 16-bit connections correctly.
        while c < conn_len {
            let resp = hda.send_command(
                HDACommand::new()
                    .with_codec_addr(codec)
                    .with_node_idx(node)
                    .with_command(VERB_GET_CONNECTION_LIST_ENTRY)
                    .with_data(c as u8),
            )?;

            for j in 0..per_resp {
                if connections.len() as u32 >= conn_len {
                    break;
                }

                let (mask, shift) = if is_long {
                    (0xFFFFu32, 16)
                } else {
                    (0xFFu32, 8)
                };
                let entry = (resp >> (j * shift)) & mask;

                let range_bit = if is_long { 0x8000 } else { 0x80 };
                let node_mask = if is_long { 0x7FFF } else { 0x7F };

                let is_range = (entry & range_bit) != 0;
                let conn_nid = (entry & node_mask) as u16;

                if is_range && !connections.is_empty() {
                    let prev = *connections.last().unwrap();
                    for r in (prev + 1)..=conn_nid {
                        connections.push(r);
                    }
                } else {
                    connections.push(conn_nid);
                }
            }

            c += per_resp;
        }

        Ok(Self {
            node_id: node,
            kind,
            raw_caps,
            connections,
        })
    }
}

#[derive(Debug)]
pub struct AudioFunctionGroup {
    widgets: Vec<Widget>,
}

impl AudioFunctionGroup {
    pub fn route_and_unmute(
        &self,
        codec: u8,
        first: &Widget,
        last: &Widget,
        hda: &IntelHDA,
    ) -> Result<Option<u8>, WaitError> {
        if let Some(idx) = first
            .connections
            .iter()
            .position(|&c| c == last.node_id as u16)
        {
            hda.send_command(
                HDACommand::new()
                    .with_codec_addr(codec)
                    .with_node_idx(first.node_id)
                    .with_command(VERB_SET_CONNECTION_SELECTOR)
                    .with_data(idx as u8),
            )?;

            if first.has_out_amp() {
                let caps = hda.get_param(codec, first.node_id, param::AMP_OUT)?;

                hda.send_long_command(
                    LongHDACommand::new()
                        .with_codec_addr(codec)
                        .with_node_idx(first.node_id)
                        .with_command(VERB_SET_AMP_GAIN_MUTE)
                        .with_data(
                            AMP_SET_OUTPUT
                                | AMP_SET_LEFT
                                | AMP_SET_RIGHT
                                | (caps as u16 & 0x7F) /* == offset == 0dB */
                                | ((idx as u16 & 0b1111) << 8),
                        ),
                )?;
            }
            return Ok(Some(first.node_id));
        }

        for (idx, &conn_id) in first.connections.iter().enumerate() {
            if let Some(w) = self.find_widget_by_id(conn_id as u8) {
                if self.route_and_unmute(codec, w, last, hda)?.is_some() {
                    hda.send_command(
                        HDACommand::new()
                            .with_codec_addr(codec)
                            .with_node_idx(first.node_id)
                            .with_command(VERB_SET_CONNECTION_SELECTOR)
                            .with_data(idx as u8),
                    )?;

                    if first.has_out_amp() {
                        let caps = hda.get_param(codec, first.node_id, param::AMP_OUT)?;

                        hda.send_long_command(
                            LongHDACommand::new()
                                .with_codec_addr(codec)
                                .with_node_idx(first.node_id)
                                .with_command(VERB_SET_AMP_GAIN_MUTE)
                                .with_data(
                                    AMP_SET_OUTPUT
                                                    | AMP_SET_LEFT
                                                    | AMP_SET_RIGHT
                                                    | (caps as u16 & 0x7F) /* == offset == 0dB */
                                                    | ((idx as u16 & 0b1111) << 8),
                                ),
                        )?;
                    }
                    return Ok(Some(first.node_id));
                }
            }
        }
        Ok(None)
    }

    pub fn find_widget_by_id(&self, id: u8) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.node_id == id)
    }

    pub fn find_widget_by_id_dac(&self, id: u8) -> Option<(&Widget, &Widget)> {
        match self.find_widget_by_id(id) {
            Some(w) if w.kind == WidgetKind::AudioOutput => Some((w, w)),
            Some(w) if w.kind == WidgetKind::Mixer || w.kind == WidgetKind::Selector => w
                .connections
                .iter()
                .find_map(|c| self.find_widget_by_id_dac((*c) as u8).map(|(_p, f)| (w, f))),
            _ => None,
        }
    }
    pub fn find_best_out_pin(&self) -> Option<(u8, PinComplex, (u8, &Widget, &Widget))> {
        let mut found: Option<(u8, PinComplex, (u8, &Widget, &Widget))> = None;
        for widget in &self.widgets {
            if let WidgetKind::PinComplex(comp) = widget.kind
                && widget.connections.len() != 0
                && comp.is_output_capable()
                && comp.port_connectivity() != PortConnectivity::None
                && matches!(
                    comp.default_device(),
                    DefaultDevice::LineOut | DefaultDevice::HPOut | DefaultDevice::Speaker
                )
            {
                let dac = widget
                    .connections
                    .iter()
                    .enumerate()
                    .find_map(|(idx, conn)| {
                        self.find_widget_by_id_dac(*conn as u8)
                            .map(|(w, o_w)| (idx as u8, w, o_w))
                    });

                if let Some(dac) = dac
                    && found.is_none_or(|(_, p, _)| {
                        p.association() == comp.association() && p.sequence() > comp.sequence()
                    })
                {
                    found = Some((widget.node_id, comp, dac))
                }
            }
        }
        found
    }

    pub fn discover(hda: &IntelHDA, codec: u8, node: u8) -> Result<Self, WaitError> {
        let nodes = hda.get_param(codec, node, param::SUB_NODE_COUNT)?;
        let w_start = (nodes >> 16) & 0xFF;
        let w_count = nodes & 0xFF;
        debug!(ihda::AudioFunctionGroup, "AFG {node} has {w_count} widgets");

        let mut widgets = Vec::with_capacity(w_count as usize);
        for w in w_start..(w_start + w_count) {
            let widget = Widget::discover(hda, codec, w as u8)?;
            widgets.push(widget);
        }

        Ok(Self { widgets })
    }
}

#[derive(Debug)]
pub struct Codec {
    addr: u8,
    afgs: Vec<AudioFunctionGroup>,
}

impl Codec {
    pub fn enumarate(hda: &IntelHDA, statests: u16) -> Result<Vec<Codec>, WaitError> {
        let mut codecs = Vec::new();
        for codec in 0..15u8 {
            if (statests >> codec) & 1 != 0 {
                codecs.push(Codec::discover(hda, codec)?);
            }
        }
        Ok(codecs)
    }

    pub fn discover(hda: &IntelHDA, codec: u8) -> Result<Codec, WaitError> {
        let nodes = hda.get_param(codec, 0, param::SUB_NODE_COUNT)?;
        let fg_start = (nodes >> 16) & 0xFF;
        let fg_total = nodes & 0xFF;

        debug!(
            ihda::Codec,
            "Codec {codec} has {fg_total} function groups starting at {fg_start}"
        );

        let mut afgs = Vec::with_capacity(fg_total as usize);
        for fg in fg_start..(fg_start + fg_total) {
            let fg_type = hda.get_param(codec, fg as u8, param::FUNCTION_GROUP_TYPE)?;

            if fg_type as u8 == AUDIO_FUNCTION_GROUP {
                let afg = AudioFunctionGroup::discover(hda, codec, fg as u8)?;
                afgs.push(afg);
            }
        }

        Ok(Self { addr: codec, afgs })
    }
}

#[derive(Debug)]
struct HDARegs {
    port: AllocatedBar,
}

/// Describes a Target Ring operation and it's offset.
#[derive(Debug, Clone, Copy)]
enum TargetRing {
    CommandRing = 0x40,
    ResponseRing = 0x50,
}

impl HDARegs {
    #[inline]
    pub fn from_bars(bars: &[AllocatedBar]) -> Option<Self> {
        bars.get(0).map(|b| Self { port: *b })
    }

    #[inline]
    pub fn gctl(&self) -> u16 {
        unsafe { self.port.read_u16(0x08) }
    }

    #[inline]
    pub fn set_gctl(&mut self, gctl: u16) {
        unsafe { self.port.write_u16(0x08, gctl) }
    }

    #[inline]
    pub fn gcap(&self) -> u16 {
        unsafe { self.port.read_u32(0x00) as u16 }
    }

    #[inline]
    pub fn version(&self) -> (u8, u8) {
        unsafe { (self.port.read_u8(0x02), self.port.read_u8(0x03)) }
    }

    #[inline]
    pub fn statests(&self) -> u16 {
        unsafe { self.port.read_u16(0xE) }
    }

    #[inline]
    pub fn set_intctl(&mut self, ctl: u32) {
        unsafe { self.port.write_u32(0x20, ctl) }
    }

    #[inline]
    pub fn set_addr(&mut self, ring: TargetRing, addr: PhysAddr) {
        unsafe {
            self.port.write_u32(ring as u16, addr.into_raw() as u32);
            self.port
                .write_u32(ring as u16 + 4, (addr.into_raw() >> 32) as u32);
        }
    }

    #[inline]
    pub fn set_wp(&mut self, ring: TargetRing, wp: u16) {
        unsafe {
            self.port.write_u16(ring as u16 + 8, wp);
        }
    }

    #[inline]
    pub fn wp(&self, ring: TargetRing) -> u16 {
        unsafe { self.port.read_u16(ring as u16 + 8) }
    }

    #[inline]
    pub fn set_corb_wp(&mut self, wp: u16) {
        self.set_wp(TargetRing::CommandRing, wp)
    }

    #[inline]
    pub fn corb_wp(&self) -> u16 {
        self.wp(TargetRing::CommandRing)
    }

    #[inline]
    pub fn set_rirb_wp(&mut self, wp: u16) {
        self.set_wp(TargetRing::ResponseRing, wp)
    }

    #[inline]
    pub fn rirb_wp(&self) -> u16 {
        self.wp(TargetRing::ResponseRing)
    }

    #[inline]
    pub fn set_rirb_intcnt(&mut self, cnt: u16) {
        unsafe {
            self.port.write_u16(0x5A, cnt);
        }
    }

    #[inline]
    pub fn rirb_intcnt(&self) -> u16 {
        unsafe { self.port.read_u16(0x5A) }
    }

    #[inline]
    pub fn set_corb_rp(&mut self, rp: u16) {
        unsafe {
            self.port.write_u16(0x4A, rp);
        }
    }

    #[inline]
    pub fn corb_rp(&self) -> u16 {
        unsafe { self.port.read_u16(0x4A) }
    }

    #[inline]
    pub fn set_ctrl(&mut self, ring: TargetRing, ctrl: u8) {
        unsafe {
            self.port.write_u8(ring as u16 + 0xC, ctrl);
        }
    }

    #[inline]
    pub fn get_ctrl(&self, ring: TargetRing) -> u8 {
        unsafe { self.port.read_u8(ring as u16 + 0xC) }
    }

    #[inline]
    pub fn conf_size(&mut self, ring: TargetRing) -> usize {
        unsafe {
            let sz_bits = self.port.read_u8(ring as u16 + 0xE);
            let s256 = sz_bits & (1 << 6) != 0;
            let s16 = sz_bits & (1 << 5) != 0;
            let s2 = sz_bits & (1 << 4) != 0;

            let (bits, sz) = if s256 {
                (0b10, 256)
            } else if s16 {
                (0b01, 16)
            } else if s2 {
                (0b00, 2)
            } else {
                (0b00, 2)
            };

            self.port.write_u8(ring as u16 + 0xE, bits);
            sz
        }
    }

    #[inline]
    pub fn rirb_sts(&self) -> u8 {
        unsafe { self.port.read_u8(0x5D) }
    }

    #[inline]
    pub fn intsts(&self) -> u32 {
        unsafe { self.port.read_u32(0x24) }
    }

    #[inline]
    pub fn set_rirb_sts(&self, value: u8) {
        unsafe { self.port.write_u8(0x5D, value) }
    }
}

#[derive(Debug, Clone, Copy)]
enum FBitsPerSample {
    /// The data will be packed in memory in 8-bit containers on 16-bit boundaries.
    B8 = 0b000,
    /// The data will be packed in memory in 16-bit containers on 16-bit boundaries.
    B16 = 0b001,
    /// The data will be packed in memory in 32-bit containers on 32-bit boundaries.
    B20 = 0b010,
    /// The data will be packed in memory in 32-bit containers on 32-bit boundaries.
    B24 = 0b011,
    /// The data will be packed in memory in 32-bit containers on 32-bit boundaries.
    B32 = 0b100,
    Rsvd,
}

impl FBitsPerSample {
    /// Returns the bits as a number.
    pub const fn to_raw(&self) -> u8 {
        match self {
            Self::B16 => 16,
            Self::B20 => 20,
            Self::B24 => 24,
            Self::B32 => 32,
            Self::B8 => 8,
            Self::Rsvd => 16, /* FIXME: Handle this correctly */
        }
    }
    /// Returns the amount of bits actually taken per sample including padding.
    pub const fn stride(&self) -> u8 {
        match self {
            Self::B16 => 16,
            Self::B20 => 32,
            Self::B24 => 32,
            Self::B32 => 32,
            Self::B8 => 16,
            Self::Rsvd => 16, /* FIXME: Handle this correctly */
        }
    }

    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0b111 {
            0b000 => Self::B8,
            0b001 => Self::B16,
            0b010 => Self::B20,
            0b011 => Self::B24,
            0b100 => Self::B32,
            _ => Self::Rsvd,
        }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy)]
enum SampleRateBase {
    Khz48 = 0,
    Khz44_1 = 1,
}

impl SampleRateBase {
    pub const fn hz(&self) -> u32 {
        match self {
            SampleRateBase::Khz44_1 => 44100,
            SampleRateBase::Khz48 => 48000,
        }
    }
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 1 {
            0b0 => Self::Khz48,
            0b1 => Self::Khz44_1,
            _ => unreachable!(),
        }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }
}

#[bitfield(u16)]
struct SndFmt {
    #[bits(4)]
    channels_minus_1: u8,
    #[bits(3)]
    bits_per_sample: FBitsPerSample,
    #[bits(1)]
    _rsvd: (),
    #[bits(3)]
    div_minus_1: u8,
    #[bits(3)]
    mult_minus_1: u8,
    #[bits(1)]
    base_rate: SampleRateBase,
    #[bits(1)]
    _rsvd1: (),
}

impl SndFmt {
    pub fn channels(&self) -> u8 {
        self.channels_minus_1() + 1
    }
    pub fn mult(&self) -> u8 {
        self.mult_minus_1() + 1
    }

    pub fn div(&self) -> u8 {
        self.div_minus_1() + 1
    }

    pub fn rate_hz(&self) -> u32 {
        self.base_rate().hz() * self.mult() as u32 / self.div() as u32
    }

    pub fn from_raw(channels: u8, rate_hz: u32, bits_per_sample: FBitsPerSample) -> Option<Self> {
        let mut found = if rate_hz == 44100 {
            Some((SampleRateBase::Khz44_1, 0, 0))
        } else if rate_hz == 48000 {
            Some((SampleRateBase::Khz48, 0, 0))
        } else {
            None
        };

        if found.is_none() {
            for div in 1..=8u32 {
                for mult in 1..=4u32 {
                    if 44100 * mult == rate_hz * div {
                        found = Some((SampleRateBase::Khz44_1, (mult - 1) as u8, (div - 1) as u8));
                        break;
                    } else if 48000 * mult == rate_hz * div {
                        found = Some((SampleRateBase::Khz48, (mult - 1) as u8, (div - 1) as u8));
                        break;
                    }
                }
            }
        }

        found.map(|(base, mul, div)| {
            Self::new()
                .with_base_rate(base)
                .with_div_minus_1(div)
                .with_mult_minus_1(mul)
                .with_bits_per_sample(bits_per_sample)
                .with_channels_minus_1(channels.saturating_sub(1) & 0b1111)
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct StreamRegs {
    port: AllocatedBar,
    base: u16,
}

impl StreamRegs {
    #[inline]
    fn at(bar: AllocatedBar, in_streams: u16, is_output: bool, index: u16) -> Self {
        let block_base = 0x80 + if is_output { in_streams * 0x20 } else { 0 } + index * 0x20;
        Self {
            port: bar,
            base: block_base,
        }
    }

    pub fn ctl(&self) -> u32 {
        unsafe { self.port.read_u32(self.base + 0x00) & 0xFFFFFF }
    } // 24-bit

    pub fn start(&mut self) {
        self.set_ctl(self.ctl() | (1 << 1));
    }
    pub fn set_ctl(&mut self, v: u32) {
        unsafe {
            self.port.write_u8(self.base + 0x00, v as u8);
            self.port.write_u8(self.base + 0x01, (v >> 8) as u8);
            self.port.write_u8(self.base + 0x02, (v >> 16) as u8);
        }
    }
    pub fn sts(&self) -> u8 {
        unsafe { self.port.read_u8(self.base + 0x03) }
    }

    pub fn set_sts(&self, sts: u8) {
        unsafe { self.port.write_u8(self.base + 0x03, sts) }
    }
    pub fn lpib(&self) -> u32 {
        unsafe { self.port.read_u32(self.base + 0x04) }
    }
    pub fn set_cbl(&mut self, v: u32) {
        unsafe { self.port.write_u32(self.base + 0x08, v) }
    }
    pub fn set_lvi(&mut self, v: u16) {
        unsafe { self.port.write_u16(self.base + 0x0C, v) }
    }
    pub fn set_fmt(&mut self, v: SndFmt) {
        unsafe { self.port.write_u16(self.base + 0x12, v.into_bits()) }
    }

    pub fn set_bdp(&mut self, v: PhysAddr) {
        self.set_bdpl(v.into_raw() as u32);
        self.set_bdpu((v.into_raw() >> 32) as u32);
    }
    pub fn set_bdpl(&mut self, v: u32) {
        unsafe { self.port.write_u32(self.base + 0x18, v) }
    }
    pub fn set_bdpu(&mut self, v: u32) {
        unsafe { self.port.write_u32(self.base + 0x1C, v) }
    }
}

fn reset(regs: &mut HDARegs) -> Result<(), ()> {
    debug!(IntelHDA, "Performing reset");
    // Is operational
    if (regs.gctl() & 1) != 0 {
        // Unset operational bit for reset
        regs.set_gctl(regs.gctl() & !1);
        // Wait for it to clear
        if !crate::sleep_until!(500 ms, (regs.gctl() & 1) == 0) {
            error!(IntelHDA, "Timeout waiting for reset");
            return Err(());
        }
    }

    // We have to sleep for a while
    crate::sleep!(100 ms);

    // Set the operational bit
    regs.set_gctl(regs.gctl() | 1);
    if !crate::sleep_until!(500 ms, (regs.gctl() & 1) != 0) {
        error!(IntelHDA, "Timeout waiting for operational");
        return Err(());
    }

    // We have to sleep for a while
    crate::sleep!(100 ms);

    Ok(())
}
fn create_buffers(regs: &mut HDARegs) -> Result<(DMABuffer<u32>, DMABuffer<u64>), ()> {
    // Stop both rings.
    regs.set_ctrl(
        TargetRing::CommandRing,
        regs.get_ctrl(TargetRing::CommandRing) & !(1 << 1),
    );

    regs.set_ctrl(
        TargetRing::ResponseRing,
        regs.get_ctrl(TargetRing::ResponseRing) & !(1 << 1),
    );

    let cmd_size = regs.conf_size(TargetRing::CommandRing);
    let resp_size = regs.conf_size(TargetRing::ResponseRing);

    let (corb, rirb) = (
        DMABuffer::new_filled(cmd_size, 0)?,
        DMABuffer::new_filled(resp_size, 0)?,
    );

    regs.set_addr(TargetRing::CommandRing, corb.phys());
    regs.set_addr(TargetRing::ResponseRing, rirb.phys());

    // Clear write pointer, (only the first byte is used).
    regs.set_corb_wp(regs.corb_wp() & !(0xFF));

    // Set the read pointer reset bit
    regs.set_corb_rp(regs.corb_rp() | 0x8000);

    // Wait for the reset to be acknowledged
    if !crate::sleep_until!(1000 ms, (regs.corb_rp() & 0x8000) != 0) {
        crate::error!(IntelHDA, "CORBPRST was never set");
    }

    // Clear the read pointer reset bit (not self clearing)
    regs.set_corb_rp(0);

    // Wait for the clear to be acknowledged
    if !crate::sleep_until!(1000 ms, (regs.corb_rp() & 0x8000) == 0) {
        crate::error!(IntelHDA, "CORBPRST was never cleared");
    }

    // Reset the rirb write pointer (always reads as zero).
    regs.set_rirb_wp(regs.rirb_wp() | 0x8000);
    // Set the interrupt count to 1.
    // (how many responses till interrupt).
    regs.set_rirb_intcnt((regs.rirb_intcnt() & 0xFF00) | 1);

    debug!(
        IntelHDA,
        "Initialized corb => {:?} with {} entries, rirb => {:?} with {} entries",
        corb.phys(),
        corb.len(),
        rirb.phys(),
        rirb.len()
    );

    regs.set_ctrl(
        TargetRing::CommandRing,
        regs.get_ctrl(TargetRing::CommandRing)
            | (1 << 0/* interrupt on memory error */)
            | (1 << 1/* enable */),
    );

    regs.set_ctrl(
        TargetRing::ResponseRing,
        regs.get_ctrl(TargetRing::ResponseRing)
            | (1 << 0/* interrupt on memory error */)
            | (1 << 1/* enable */),
    );

    crate::serial!(
        "CORBCTL: {:#x}, RIRBCTL: {:#x}\n",
        regs.get_ctrl(TargetRing::CommandRing),
        regs.get_ctrl(TargetRing::ResponseRing)
    );
    Ok((corb, rirb))
}

// FIXME: For now we find the best output to use to bump audio, later we have to do multiple outputs, hardware mixers etc...
fn find_output_target(
    codecs: &[Codec],
) -> Option<(
    u8, /* codec */
    &AudioFunctionGroup,
    (u8, &Widget, &Widget), /* DAC node parent, DAC end */
    (u8, PinComplex),
)> {
    let mut found = None;
    for codec in codecs {
        for afg in &codec.afgs {
            if let Some((comp_id, comp, dac)) = afg.find_best_out_pin() {
                found = Some((codec.addr, afg, dac, (comp_id, comp)));
                break;
            }
        }
    }
    found
}

const RATE_PREFERENCE: &[(u32, u8)] = &[
    (44100, 5),
    (48000, 6),
    (88200, 7),
    (176400, 9),
    (96000, 8),
    (192000, 10),
    (32000, 4),
    (22050, 3),
    (16000, 2),
    (11025, 1),
    (8000, 0),
];

fn pick_rate(supported_mask: u32) -> u32 {
    RATE_PREFERENCE
        .iter()
        .find(|(_, bit)| (supported_mask >> bit) & 1 != 0)
        .map(|(rate, _)| *rate)
        .unwrap_or(48000 /* always supported */)
}

fn pick_bits_per_sample(supported_mask: u32) -> FBitsPerSample {
    if supported_mask & (1 << 19) != 0 {
        FBitsPerSample::B24
    } else if supported_mask & (1 << 20) != 0 {
        FBitsPerSample::B32
    } else {
        FBitsPerSample::B16
    }
}

fn init_next_out(
    hda: &IntelHDA,
    codec: u8,
    afg: &AudioFunctionGroup,
    (pin_node, pin): (u8, PinComplex),
    (dac_parent_idx, dac_parent, dac): (u8, &Widget, &Widget),
) -> Result<u16, &'static str> {
    afg.route_and_unmute(codec, dac_parent, dac, hda)
        .map_err(|_| "Timeout connecting between pin and DAC")?;

    let caps = hda
        .get_param(codec, dac.node_id, param::AMP_OUT)
        .map_err(|_| "Timeout reading DAC amp caps")?;
    hda.send_long_command(
        LongHDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(dac.node_id)
            .with_command(VERB_SET_AMP_GAIN_MUTE)
            .with_data(AMP_SET_OUTPUT | AMP_SET_LEFT | AMP_SET_RIGHT | (caps as u16 & 0x7F)),
    )
    .map_err(|_| "Timeout unmuting DAC")?;

    hda.send_command(
        HDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(pin_node)
            .with_command(VERB_SET_CONNECTION_SELECTOR)
            .with_data(dac_parent_idx),
    )
    .map_err(|_| "Timeout connecting between pin and DAC")?;

    let caps = hda
        .get_param(codec, pin_node, param::AMP_OUT)
        .map_err(|_| "Timeout unmuting Pin")?;
    hda.send_long_command(
        LongHDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(pin_node)
            .with_command(VERB_SET_AMP_GAIN_MUTE)
            .with_data(
                AMP_SET_OUTPUT
                    | AMP_SET_LEFT
                    | AMP_SET_RIGHT
                    | (caps as u16 & 0x7F)
                    | ((dac_parent_idx as u16 & 0b1111) << 8),
            ),
    )
    .map_err(|_| "Timeout unmuting Pin")?;
    const FRAMES_BASE: u32 = 1024;
    const PERIODS_COUNT: u32 = 3;

    let supported_pcm = hda
        .get_param(codec, dac.node_id, param::SUPPORTED_PCM_SIZE_RATES)
        .map_err(|_| "Timeout waiting for command")?;
    let best_rate = pick_rate(supported_pcm);
    let best_bits_per_sample = pick_bits_per_sample(supported_pcm);
    let fmt = SndFmt::from_raw(
        if dac.supports_stereo() { 2 } else { 1 },
        best_rate,
        best_bits_per_sample,
    )
    .expect("We should be picking a valid rate/samples");
    debug!(
        IntelHDA,
        "Creating a new output with format: {fmt:#?}, pin: {pin:#?}"
    );

    let frames_per_period = (fmt.rate_hz() / 44100) * FRAMES_BASE;
    let bytes_per_period =
        frames_per_period * ((fmt.bits_per_sample().stride() / 8) as u32) * fmt.channels() as u32;
    let bytes_total = bytes_per_period * PERIODS_COUNT;

    let total_bytes = bytes_total.next_multiple_of(PAGE_SIZE as u32) as usize;
    let dma = DMABuffer::<u8>::new_filled(total_bytes, 0).map_err(|_| "Out of memory")?;
    let mut bds = DMABuffer::<BD>::new(PERIODS_COUNT as usize).map_err(|_| "Out of memory")?;
    let base = dma.phys();

    for i in 0..PERIODS_COUNT {
        bds.push(BD {
            addr: base + (i as usize * bytes_per_period as usize),
            len: bytes_per_period as u32,
            config: 1,
        })
        .expect("Should be unreachable as we allocate and push PERIOD_COUNT");
    }

    let stream_idx = hda
        .next_out_stream
        .fetch_update(
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
            |i| ((i + 1) < hda.out_streams).then_some(i + 1),
        )
        .map_err(|_| "No output stream found")?;

    let stream_tag = stream_idx + 1;
    let mut regs =
        unsafe { StreamRegs::at((*hda.regs.get()).port, hda.in_streams, true, stream_idx) };

    debug!(ihda::Stream, "Reseting stream: {stream_idx}");
    regs.set_ctl(regs.ctl() | 1);
    if !crate::sleep_until!(500 ms, (regs.ctl() & 1) != 0) {
        return Err("Timeout waiting for stream reset set");
    }
    regs.set_ctl(regs.ctl() & !1);
    if !crate::sleep_until!(500 ms, (regs.ctl() & 1) == 0) {
        return Err("Timeout waiting for stream reset clear");
    }

    // Generate interrupts if IOC bit is set and on descriptor error.
    regs.set_ctl((regs.ctl() & 0x00FF00) | (1 << 2) | (1 << 4) | ((stream_tag as u32) << 4 << 16));
    regs.set_fmt(fmt);
    regs.set_bdp(bds.phys());
    regs.set_cbl(total_bytes as u32);
    regs.set_lvi((bds.len() - 1) as u16);

    hda.send_command(
        HDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(dac.node_id)
            .with_command(VERB_SET_CONVERTOR_CHANNEL)
            .with_data(((stream_tag as u8) << 4) | 0 /* channel 0 */),
    )
    .map_err(|_| "Timeout mapping stream to DAC")?;

    hda.send_long_command(
        LongHDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(dac.node_id)
            .with_command(VERB_SET_CONVERTOR_FMT)
            .with_data(fmt.into_bits()),
    )
    .map_err(|_| "Timeout setting DAC fmt")?;

    hda.send_command(
        HDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(pin_node)
            .with_command(VERB_SET_PIN_CTL)
            .with_data(1 << 6),
    )
    .map_err(|_| "Timeout enabling pin")?;

    // Putting pin in the highest power state.
    hda.send_command(
        HDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(pin_node)
            .with_command(VERB_SET_EAPD_BTL_ENB)
            .with_data(1 << 1),
    )
    .map_err(|_| "Timeout setting EAPD")?;

    hda.send_command(
        HDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(pin_node)
            .with_command(VERB_SET_POWER_STATE)
            .with_data(0x0),
    )
    .map_err(|_| "Timeout setting Pin power state")?;

    hda.send_command(
        HDACommand::new()
            .with_codec_addr(codec)
            .with_node_idx(dac.node_id)
            .with_command(VERB_SET_POWER_STATE)
            .with_data(0x0),
    )
    .map_err(|_| "Timeout setting DAC power state")?;

    let format = AudioInfo::new(
        fmt.rate_hz(),
        fmt.bits_per_sample().to_raw(),
        fmt.bits_per_sample().stride(),
        fmt.channels(),
    );
    let stream = Arc::new(Stream {
        queued_periods: SpinLock::new((
            Vec::with_capacity(total_bytes),
            VecDeque::with_capacity(PERIODS_COUNT as usize),
        )),
        tag: stream_tag,
        _bdl: bds,
        audio_buf: SyncUnsafeCell::new(dma),
        format,
        regs: SyncUnsafeCell::new(regs),
        write_pointer: SyncUnsafeCell::new(0),
        writers: SpinLock::new(WaitQueue::new()),
    });

    without_interrupts(|| hda.streams.lock().push(stream.clone()));

    debug!(IntelHDA, "Registering stream: {}", stream.tag);
    audio::register_stream(
        &alloc::format!("iHDA{}x1", stream.tag),
        Box::new(stream.clone()),
    );

    crate::process::current::kernel_thread_spawn(
        ihda_snd_thread,
        unsafe { &*Arc::into_raw(stream) },
        None,
        None,
    )
    .expect("Failed to summon stream's thread");

    Ok(stream_tag)
}

#[bitfield(u32)]
struct HDACommand {
    data: u8,
    #[bits(12)]
    command: u16,
    node_idx: u8,
    #[bits(4)]
    codec_addr: u8,
}

#[bitfield(u32)]
struct LongHDACommand {
    data: u16,
    #[bits(4)]
    command: u8,
    node_idx: u8,
    #[bits(4)]
    codec_addr: u8,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct BD {
    addr: PhysAddr,
    len: u32,
    config: u32,
}

const _: () = assert!(size_of::<BD>() == 16);

fn ihda_snd_thread(_tid: Tid, stream: &'static Stream) -> ! {
    let stream = unsafe { Arc::from_raw(stream) };

    unsafe { &mut (*stream.regs.get()) }.start();
    loop {
        without_interrupts(|| {
            stream.stream_next_periods();

            let wait = unsafe { stream.writers.prepare_wait() };
            if stream.current_space() >= stream.period_bytes() {
                return;
            }

            wait.enter_wait((), None)
                .expect("Failed to wait for stream to have free space")
        })
    }
}
#[derive(Debug)]
struct Stream {
    tag: u16,
    regs: SyncUnsafeCell<StreamRegs>,
    _bdl: DMABuffer<BD>,
    audio_buf: SyncUnsafeCell<DMABuffer<u8>>,
    queued_periods: SpinLock<(Vec<u8>, VecDeque<usize>)>,
    write_pointer: SyncUnsafeCell<usize>,
    writers: SpinLock<WaitQueue<1>>,

    format: AudioInfo,
}

impl Stream {
    fn periods_count(&self) -> usize {
        self._bdl.len()
    }

    fn total_bytes(&self) -> usize {
        unsafe { (&*self.audio_buf.get()).len() }
    }

    fn period_bytes(&self) -> usize {
        self.total_bytes() / self.periods_count()
    }

    fn current_space(&self) -> usize {
        unsafe {
            self.space(
                *self.write_pointer.get(),
                (*self.regs.get()).lpib() as usize,
            )
        }
    }
    fn space(&self, head: usize, tail: usize) -> usize {
        let size = self.total_bytes();
        let head = head % size;
        let tail = tail % size;

        (size + tail - head - 1) % size
    }

    fn ac_queued_bytes(&self) -> usize {
        without_interrupts(|| self.queued_periods.lock().0.len())
    }

    fn fill_next(&self, with: u8, len: usize) -> usize {
        if len == 0 {
            return 0;
        }

        let regs = unsafe { &mut (*self.regs.get()) };
        let head = unsafe { &mut *self.write_pointer.get() };

        let tail = regs.lpib() as usize;
        let buf = unsafe { &mut *self.audio_buf.get() };
        let size = buf
            .len()
            .to_previous_multiple_of(self.format.bytes_per_frame());
        let space = self
            .space(*head, tail)
            .to_previous_multiple_of(self.format.bytes_per_frame());

        if space == 0 {
            return 0;
        }

        let to_copy = len.min(space);
        let to_copy_start = to_copy.min(size - (*head % size));
        let to_copy_end = to_copy - to_copy_start;

        buf[*head % size..(*head % size) + to_copy_start].fill(with);
        buf[0..to_copy_end].fill(with);

        *head = head.wrapping_add(to_copy);
        to_copy
    }
    fn write_data(&self, data: &[u8]) -> usize {
        if data.len() == 0 {
            return 0;
        }

        let regs = unsafe { &mut (*self.regs.get()) };
        let head = unsafe { &mut *self.write_pointer.get() };

        let tail = regs.lpib() as usize;
        let buf = unsafe { &mut *self.audio_buf.get() };
        let size = buf
            .len()
            .to_previous_multiple_of(self.format.bytes_per_frame());
        let space = self
            .space(*head, tail)
            .to_previous_multiple_of(self.format.bytes_per_frame());

        if space == 0 {
            crate::serial!("space == 0\n");
            return 0;
        }

        let to_copy = data.len().min(space);
        let to_copy_start = to_copy.min(size - (*head % size));
        let to_copy_end = to_copy - to_copy_start;

        buf[*head % size..(*head % size) + to_copy_start].copy_from_slice(&data[..to_copy_start]);
        buf[0..to_copy_end].copy_from_slice(&data[to_copy_start..(to_copy_start + to_copy_end)]);

        *head = head.wrapping_add(to_copy);
        to_copy
    }

    fn queue_data(&self, data: &[u8]) {
        if data.len() == 0 {
            return;
        }

        without_interrupts(|| {
            let mut queue_guard = self.queued_periods.lock();
            let (queued_data, queued_periods) = &mut *queue_guard;
            for chunk in data.chunks(self.period_bytes()) {
                queued_data.extend_from_slice(chunk);
                let mut end_len = chunk.len();
                if let Some(last_len) = queued_periods.back_mut()
                    && self.period_bytes() > *last_len
                {
                    let tmp = *last_len;
                    *last_len += chunk.len();
                    *last_len = (*last_len).min(self.period_bytes());
                    end_len = end_len - (*last_len - tmp);
                }

                if end_len != 0 {
                    queued_periods.push_back(end_len);
                }
            }
        });
    }

    // FIXME: Do this in a thread instead of the interrupt handler.
    fn stream_next_periods(&self) -> bool {
        let mut queue_guard = self.queued_periods.lock();
        let (queued_data, queued_periods) = &mut *queue_guard;

        let Some(len) = queued_periods.pop_front() else {
            self.fill_next(0, self.period_bytes());
            return false;
        };

        self.write_data(&queued_data.drain(..len).as_slice()[..len]);
        if self.period_bytes() > len {
            // self.fill_next(0, self.period_bytes() - len);
            return true;
        }

        while let Some(len) = queued_periods.pop_front() {
            if len < self.period_bytes() || self.current_space() < self.period_bytes() {
                queued_periods.push_front(len);
                break;
            } else {
                let wrote = self.write_data(&queued_data.drain(..len).as_slice()[..len]);
                if len > wrote {
                    queued_periods.push_front(len - wrote);
                    break;
                }
            }
        }

        true
    }

    fn on_period_elapsed(&self) {
        self.writers.lock().wake_all();
    }
}

unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}
impl AudioCard for Arc<Stream> {
    fn name(&self) -> &'static str {
        "iHDA"
    }

    fn info(&self) -> AudioInfo {
        self.format
    }

    fn transfer_buf_size(&self) -> usize {
        self.total_bytes()
    }

    fn queued_samples_count(&self) -> usize {
        self.ac_queued_bytes() / self.format.bytes_per_sample()
    }

    fn transfer_data(&self, data: &[u8]) -> FSResult<usize> {
        self.queue_data(data);
        Ok(data.len())
    }
}

#[derive(Debug)]
pub struct IntelHDA {
    regs: SyncUnsafeCell<HDARegs>,
    corb: SpinLock<(DMABuffer<u32>, u16)>,
    rirb: SyncUnsafeCell<(DMABuffer<u64>, u16)>,
    irq: IRQInfo,
    cmd_queue: SpinLock<WaitQueueWithTimeout<1, ()>>,
    in_streams: u16,
    out_streams: u16,
    next_out_stream: AtomicU16,
    streams: SpinLock<Vec<Arc<Stream>>>,
}

impl IntelHDA {
    pub fn get_param(&self, codec: u8, node: u8, param: u8) -> Result<u32, WaitError> {
        self.send_command(
            HDACommand::new()
                .with_codec_addr(codec)
                .with_command(VERB_GET_PARAMETER)
                .with_node_idx(node)
                .with_data(param),
        )
    }

    fn send_long_command(&self, command: LongHDACommand) -> Result<u32, WaitError> {
        self.send_command_internal(command.into_bits())
    }

    fn send_command(&self, command: HDACommand) -> Result<u32, WaitError> {
        self.send_command_internal(command.into_bits())
    }

    fn send_command_internal(&self, command: u32) -> Result<u32, WaitError> {
        without_interrupts(|| {
            let wait = unsafe { self.cmd_queue.prepare_wait() };

            let regs = unsafe { &mut *self.regs.get() };
            let mut guard = self.corb.lock();
            let (corb, index) = &mut *guard;

            corb[*index as usize] = command;
            regs.set_corb_wp((regs.corb_wp() & 0xFF00) | *index);
            *index = (*index + 1) % corb.len() as u16;

            drop(guard);
            wait.enter_wait((), NonZero::new(1000))?;

            let guard = self.corb.lock();
            // FIXME: Naive implementation that can RC
            let (rirb, index) = unsafe { &mut *self.rirb.get() };

            let results = rirb[*index as usize];
            *index = (*index + 1) % rirb.len() as u16;

            drop(guard);
            Ok(results as u32)
        })
    }
}

impl InterruptReceiver for IntelHDA {
    fn handle_interrupt(&'static self) -> bool {
        let regs = unsafe { &mut *self.regs.get() };

        let mut handled = false;
        if regs.rirb_sts() != 0 {
            regs.set_rirb_sts(regs.rirb_sts());
            self.cmd_queue.lock().wake_n_on_condition(|()| true, 1);
            handled = true;
        }

        let intsts = regs.intsts();
        if intsts != 0 {
            let streams = self.streams.lock();

            for i in 0..30 {
                if intsts & (1 << i) != 0 {
                    // i is the stream index like the stream offset so
                    // for an output stream stream 0 would be at in_streams_count
                    // FIXME: Before adding input streams and such fix this
                    if let Some(stream) = streams.iter().find(|s| s.tag - 1 + self.in_streams == i)
                    {
                        let stream_regs = unsafe { &mut *stream.regs.get() };
                        let sts = stream_regs.sts();

                        if sts & (1 << 2) != 0 {
                            stream.on_period_elapsed();
                        }

                        // RW1C
                        // Clear BSICS(1) FIFO(3) error and Descriptor(4) error
                        stream_regs.set_sts(sts & ((1 << 2) | (1 << 4) | (1 << 3)));
                    }
                }
            }
            handled = true;
        }

        handled
    }
}

impl PCIDevice for IntelHDA {
    const CLASS_SUBCLASS: (u8, u8) = (0x4, 0x3);
    fn create(mut info: crate::drivers::pci::PCIDeviceInfo) -> Result<Self, &'static str>
    where
        Self: Sized,
    {
        let general_header = info.unwrap_general();
        write_ref!(
            general_header.common.command,
            PCICommandReg::BUS_MASTER | PCICommandReg::MEM_SPACE
        );
        let bars = general_header.get_bars();
        debug!(IntelHDA, "info: {info:#?}, bars: {bars:#?}");

        // FIXME: Return an error on failure and free memory.
        let allocated_bars = AllocatedBar::allocate_bars::<6>(&"IntelHDA", &*bars).0;
        let mut regs =
            HDARegs::from_bars(&*allocated_bars).ok_or("Failed to locate iHDA registers")?;
        let irq = info
            .get_best_irq_info(&*allocated_bars)
            .ok_or("Failed to allocate IRQ")?;

        let gcap = regs.gcap();
        let statests = regs.statests();

        let bit64_support = (gcap & 1) != 0;
        if !bit64_support {
            error!(IntelHDA, "iHDA device doesn't support 64bit addressing");
            return Err("No 64bit support");
        }

        let bi_streams = (gcap >> 3) & 0x1F;
        let in_streams = (gcap >> 8) & 0x0F;
        let out_streams = (gcap >> 12) & 0x0F;

        let (minor, major) = regs.version();
        info!(
            IntelHDA,
            "Version {major}.{minor}, statests: {statests:#x}, {in_streams} input streams, {out_streams} output streams, {bi_streams} bi streams"
        );

        reset(&mut regs).map_err(|_| "Failed to reset iHDA")?;
        let (corb, rirb) =
            create_buffers(&mut regs).map_err(|()| "iHDA failed to construct com rings")?;
        Ok(Self {
            corb: SpinLock::new((corb, 1)),
            rirb: SyncUnsafeCell::new((rirb, 1)),
            regs: SyncUnsafeCell::new(regs),
            cmd_queue: SpinLock::new(WaitQueueWithTimeout::new()),
            in_streams,
            out_streams,
            next_out_stream: AtomicU16::new(0),
            streams: SpinLock::new(Vec::with_capacity(1)),
            irq,
        })
    }

    fn start(&'static self) -> bool {
        debug!(IntelHDA, "Enabling interrupts: {:#?}", self.irq);
        interrupts::register_irq(self.irq.clone(), interrupts::IntTrigger::LevelAssert, self);
        let regs = unsafe { &mut *self.regs.get() };
        regs.set_intctl(u32::MAX);

        debug!(IntelHDA, "SATESTS: {:#x}", regs.statests());
        let Ok(codecs) = Codec::enumarate(self, regs.statests()) else {
            error!(IntelHDA, "Timeout enumarating Codecs");
            return false;
        };

        debug!(IntelHDA, "Codecs: {codecs:#?}");
        if let Some((codec, afg, dac, pin)) = find_output_target(&codecs) {
            if let Err(e) = init_next_out(self, codec, afg, pin, dac) {
                error!(IntelHDA, "Failed to create an output stream: {e}");
                return false;
            }

            true
        } else {
            warn!(IntelHDA, "No output found");
            false
        }
    }
}
