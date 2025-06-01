use crate::{PhysAddr, VirtAddr};
use bitflags::bitflags;
use core::fmt::Display;

#[repr(C)]
pub struct CapsReg {
    reg_length: u8,
    _reserved0: u8,
    version_number: u8,
    hcsparams_1: u32,
    hcsparams_2: u32,
    hcsparams_3: u32,
    hccparams_1: u32,
    dorbell_off: u32,
    runtime_off: u32,
    hccparams_2: u32,
}

impl CapsReg {
    pub fn operational_regs_mut(&mut self) -> &mut OperationalRegs {
        let caps_ptr = self as *const _ as *const u8;
        unsafe {
            let ptr = caps_ptr.add(self.reg_length as usize);
            &mut *(ptr as *mut OperationalRegs)
        }
    }

    pub const fn max_device_slots(&self) -> usize {
        (self.hcsparams_1 & 0xFF) as usize
    }
    pub const fn max_interrupts(&self) -> u8 {
        (self.hcsparams_1 >> 8) as u8
    }
    pub const fn max_ports(&self) -> u8 {
        (self.hcsparams_1 >> 24) as u8
    }
    pub const fn interrupt_schd_t(&self) -> u8 {
        (self.hcsparams_2 as u8) & 0xF
    }
    pub const fn erst_max(&self) -> u8 {
        ((self.hcsparams_2 >> 4) as u8) & 0xF
    }
    pub const fn max_scratchpad_buffers(&self) -> usize {
        (((self.hcsparams_2 >> 21) as u8) & 0x1F) as usize
    }
    pub const fn addressing_64bits(&self) -> bool {
        (self.hccparams_1 & 0x1) != 0
    }
    pub const fn bandwidth_negotiation(&self) -> bool {
        ((self.hccparams_1 >> 1) & 0x1) != 0
    }
    pub const fn context_sz_64bytes(&self) -> bool {
        ((self.hccparams_1 >> 2) & 0x1) != 0
    }
    pub const fn port_power_ctrl(&self) -> bool {
        ((self.hccparams_1 >> 3) & 0x1) != 0
    }
    pub const fn port_indicator_ctrl(&self) -> bool {
        ((self.hccparams_1 >> 4) & 0x1) != 0
    }
    pub const fn light_reset_support(&self) -> bool {
        ((self.hccparams_1 >> 5) & 0x1) != 0
    }
}

impl Display for CapsReg {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "XHCI Captabilites Register @{:?}:", VirtAddr::from_ptr(self as *const _))?;
        writeln!(f, "\tLength                            : {:#x}", self.reg_length)?;
        writeln!(f, "\tMax Device Slots                  : {}", self.max_device_slots())?;
        writeln!(f, "\tMax Interrupts                    : {}", self.max_interrupts())?;
        writeln!(f, "\tMax Ports                         : {}", self.max_ports())?;
        writeln!(f, "\tIST                               : {}", self.interrupt_schd_t())?;
        writeln!(f, "\tERST Max Size                     : {}", self.erst_max())?;
        writeln!(f, "\tScratchpad Buffers                : {}", self.max_scratchpad_buffers())?;
        writeln!(f, "\t64-bit Addressing                 : {}" ,self.addressing_64bits())?;
        writeln!(f, "\tBandwidth Negotiation Implemented : {}", self.bandwidth_negotiation())?;
        writeln!(f, "\t64-byte Context Size              : {}", self.context_sz_64bytes())?;
        writeln!(f, "\tPort Power Control                : {}", self.port_power_ctrl())?;
        writeln!(f, "\tPort Indicators Control           : {}", self.port_indicator_ctrl())?;
        write!(f,   "\tLight Reset Available             : {}", self.light_reset_support())
    }
}

bitflags! {
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub struct USBCmd: u32 {
        /**
        # General Info
        - Run/Stop (R/S)
        - RW
        - Default = ‘0’, ‘1’ = Run. ‘0’ = Stop
        # Description
        > xHci Spec Section 5.4.1 USB Table 5-20: USB Command Register Bit Definitions (USBCMD) (page 358)

        When set to a ‘1’, the xHC proceeds with execution of the schedule. The xHC continues execution as long as this bit is set to a ‘1’. When this bit
        is cleared to ‘0’, the xHC completes any current or queued commands or TDs, and any USB transactions
        associated with them, then halts.
        Refer to section 5.4.1.1 for more information on how R/S shall be managed.
        The xHC shall halt within 16 ms. after software clears the Run/Stop bit if the above conditions have
        been met.
        The HCHalted (HCH) bit in the USBSTS register indicates when the xHC has finished its pending
        pipelined transactions and has entered the stopped state. Software shall not write a ‘1’ to this flag
        unless the xHC is in the Halted state (that is, HCH in the USBSTS register is ‘1’). Doing so may yield
        undefined results. Writing a ‘0’ to this flag when the xHC is in the Running state (that is, HCH = ‘0’) and
        any Event Rings are in the Event Ring Full state (refer to section 4.9.4) may result in lost events.
        When this register is exposed by a Virtual Function (VF), this bit only controls the run state of the xHC
        instance presented by the selected VF. Refer to section 8 for more information.
        */
        const RUN = 1 << 0;
        /**
        # General Info
        - Host Controller Reset (HCRST)
        - RW
        - Default = ‘0’
        # Description
        > xHci Spec Section 5.4.1 USB Table 5-20: USB Command Register Bit Definitions (USBCMD) (page 358)

        This control bit is used by software to reset the host controller.
        The effects of this bit on the xHC and the Root Hub registers are similar to a Chip Hardware Reset.

        When software writes a ‘1’ to this bit, the Host Controller resets its internal pipelines, timers, counters,
        state machines, etc. to their initial value. Any transaction currently in progress on the USB is
        immediately terminated. A USB reset shall not be driven on USB2 downstream ports, however a Hot or
        Warm Reset79 shall be initiated on USB3 Root Hub downstream ports.
        PCI Configuration registers are not affected by this reset. All operational registers, including port
        registers and port state machines are set to their initial values. Software shall reinitialize the host
        controller as described in Section 4.2 in order to return the host controller to an operational state.
        This bit is cleared to ‘0’ by the Host Controller when the reset process is complete. Software cannot
        terminate the reset process early by writing a ‘0’ to this bit and shall not write any xHC Operational or
        Runtime registers until while HCRST is ‘1’. Note, the completion of the xHC reset process is not gated by
        the Root Hub port reset process.
        Software shall not set this bit to ‘1’ when the HCHalted (HCH) bit in the USBSTS register is a ‘0’.
        Attempting to reset an actively running host controller may result in undefined behavior.
        When this register is exposed by a Virtual Function (VF), this bit only resets the xHC instance presented
        by the selected VF. Refer to section 8 for more information.
        */
        const HCRESET = 1 << 1;
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub struct USBSts: u32 {
        /**
        # General Info
        - HCHalted (HCH)
        - RO
        - Default = ‘1’
        # Description
        > xHci Spec Section 5.4.2 Table 5-21: USB Status Register Bit Definitions (USBSTS) (page 362)

        This bit is a ‘0’ whenever the Run/Stop (R/S) bit is a ‘1’. The xHC sets this bit to ‘1’ after it has stopped executing as a result of the
        Run/Stop (R/S) bit being cleared to ‘0’, either by software or by the xHC hardware
        (for example, internal error).
        If this bit is '1', then SOFs, microSOFs, or Isochronous Timestamp Packets (ITP) shall
        not be generated by the xHC, and any received Transaction Packet shall be dropped.
        When this register is exposed by a Virtual Function (VF), this bit only reflects the
        Halted state of the xHC instance presented by the selected VF. Refer to section 8 for
        more information.
        */
        const HCHALTED = 1 << 0;
        /**
        # General Info
        - Controller Not Ready (CNR)
        - RO
        - Default = ‘1’. ‘0’ = Ready and ‘1’ = Not Ready
        # Description
        > xHci Spec Section 5.4.2 Table 5-21: USB Status Register Bit Definitions (USBSTS) (page 363)

        Software shall not write any Doorbell or Operational register of the xHC, other than
        the USBSTS register, until CNR = ‘0’. This flag is set by the xHC after a Chip
        Hardware Reset and cleared when the xHC is ready to begin accepting register
        writes. This flag shall remain cleared (‘0’) until the next Chip Hardware Reset.
        */
        const NOT_READY = 1 << 11;
        /**
        # General Info
        - Host Controller Error (HCE)
        - RO
        - Default = 0. 0’ = No internal xHC error
        # Description
        > xHci Spec Section 5.4.2 Table 5-21: USB Status Register Bit Definitions (USBSTS) (page 363)

        conditions exist and ‘1’ = Internal xHC error condition. This flag shall be set to
        indicate that an internal error condition has been detected which requires software to
        reset and reinitialize the xHC. Refer to section 4.24.1 for more information.
        */
        const HCERROR = 1 << 12;
    }
}

#[repr(C)]
pub struct OperationalRegs {
    pub usbcmd: USBCmd,
    pub usbstatus: USBSts,
    page_size: u32,
    _reserved0: [u32; 2],
    pub dnctrl: u32,
    pub crcr: usize,
    _reserved1: [u32; 4],
    pub dcbaap: PhysAddr,
    pub config: u32,
    _reserved2: [u32; 49],
}

impl Display for OperationalRegs {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "XHCI Operational Registers @{:?}:", VirtAddr::from_ptr(self as *const _))?;
        writeln!(f, "\tusbcmd    : {:?}", self.usbcmd)?;
        writeln!(f, "\tusbstatus : {:?}", self.usbstatus)?;
        writeln!(f, "\tPage Size : {:#x}", self.page_size)?;
        writeln!(f, "\tdnctrl    : {:#x}", self.dnctrl)?;
        writeln!(f, "\tcrcr      : {:#x}", self.crcr)?;
        writeln!(f, "\tdcaap     : {:?}", self.dcbaap)?;
        write!(f,   "\tconfig    : {:#x}", self.config)?;
        Ok(())
    }
}
