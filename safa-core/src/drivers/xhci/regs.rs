use crate::{
    PhysAddr, VirtAddr, debug,
    drivers::xhci::{
        devices::XHCIDeviceCtx32,
        rings::{command::XHCICommandRing, event::XHCIEventRing},
        utils::allocate_buffers_frame,
    },
    memory::{
        AlignTo,
        frame_allocator::{self, Frame},
        paging::PAGE_SIZE,
    },
    sleep, sleep_until, warn,
};
use bitflags::bitflags;
use core::fmt::Display;

#[derive(derive_mmio::Mmio)]
#[repr(C)]
pub struct CapsReg {
    #[mmio(PureRead)]
    reg_length: u8,
    _reserved0: u8,
    #[mmio(PureRead)]
    version_number: u8,
    _reserved1: u8,
    #[mmio(PureRead)]
    hcsparams_1: u32,
    #[mmio(PureRead)]
    hcsparams_2: u32,
    #[mmio(PureRead)]
    hcsparams_3: u32,
    #[mmio(PureRead)]
    hccparams_1: u32,
    #[mmio(PureRead)]
    doorbell_off: u32,
    #[mmio(PureRead)]
    runtime_off: u32,
    #[mmio(PureRead)]
    hccparams_2: u32,
}

impl CapsReg {}

impl<'a> MmioCapsReg<'a> {
    pub fn operational_regs(&self) -> MmioOperationalRegs<'static> {
        unsafe {
            let caps_ptr = self.ptr().cast::<u8>();
            let ptr = caps_ptr.add(self.read_reg_length() as usize);
            OperationalRegs::new_mmio(ptr.cast())
        }
    }

    pub fn runtime_regs(&self) -> MmioRuntimeRegs<'static> {
        unsafe {
            let caps_ptr = self.ptr().cast::<u8>();
            let ptr = caps_ptr.add(self.read_runtime_off() as usize);
            RuntimeRegs::new_mmio(ptr.cast())
        }
    }

    pub fn doorbells_base(&self) -> *mut DoorbellRegister {
        unsafe {
            let caps_ptr = self.ptr().cast::<u8>();
            caps_ptr
                .add(self.read_doorbell_off() as usize)
                .cast::<DoorbellRegister>()
        }
    }

    pub fn max_device_slots(&self) -> usize {
        (self.read_hcsparams_1() & 0xFF) as usize
    }
    pub fn max_interrupts(&self) -> u8 {
        (self.read_hcsparams_1() >> 8) as u8
    }
    pub fn max_ports(&self) -> u8 {
        (self.read_hcsparams_1() >> 24) as u8
    }
    pub fn interrupt_schd_t(&self) -> u8 {
        (self.read_hcsparams_2() as u8) & 0xF
    }
    pub fn erst_max(&self) -> u8 {
        ((self.read_hcsparams_2() >> 4) as u8) & 0xF
    }
    pub fn max_scratchpad_buffers(&self) -> usize {
        (((self.read_hcsparams_2() >> 21) as u8) & 0x1F) as usize
    }
    pub fn addressing_64bits(&self) -> bool {
        (self.read_hccparams_1() & 0x1) != 0
    }
    pub fn bandwidth_negotiation(&self) -> bool {
        ((self.read_hccparams_1() >> 1) & 0x1) != 0
    }
    pub fn context_sz_64bytes(&self) -> bool {
        ((self.read_hccparams_1() >> 2) & 0x1) != 0
    }
    pub fn port_power_ctrl(&self) -> bool {
        ((self.read_hccparams_1() >> 3) & 0x1) != 0
    }
    pub fn port_indicator_ctrl(&self) -> bool {
        ((self.read_hccparams_1() >> 4) & 0x1) != 0
    }
    pub fn light_reset_support(&self) -> bool {
        ((self.read_hccparams_1() >> 5) & 0x1) != 0
    }
}

impl<'a> Display for MmioCapsReg<'a> {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "XHCI Captabilites Register @{:?}:", VirtAddr::from_ptr(unsafe {self.ptr()}))?;
        writeln!(f, "\tLength                            : {:#x}", self.read_reg_length())?;
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
        /**
        # General Info
        - Interrupter Enable (INTE)
        - RW
        - Default = ‘0’
        # Description
        > xHci Spec Section 5.4.1 USB Table 5-20: USB Command Register Bit Definitions (USBCMD) (page 359)

        This bit provides system software with a means of enabling or disabling the host system interrupts generated by Interrupters. When this bit is a ‘1’, then
        Interrupter host system interrupt generation is allowed, for example, the xHC shall issue an interrupt at
        the next interrupt threshold if the host system interrupt mechanism (for example, MSI, MSI-X, etc.) is
        enabled. The interrupt is acknowledged by a host system interrupt specific mechanism.
        When this register is exposed by a Virtual Function (VF), this bit only enables the set of Interrupters
        assigned to the selected VF. Refer to section 7.7.2 for more information.
        */
        const INTERRUPT_ENABLE = 1 << 2;
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
        - Event Interrupt (EINT)
        - RW1C
        - Default = ‘0’
        # Description
        > xHci Spec Section 5.4.2 Table 5-21: USB Status Register Bit Definitions (USBSTS) (page 362)

        The xHC sets this bit to ‘1’ when the Interrupt Pending (IP) bit of any Interrupter transitions from ‘0’ to ‘1’. Refer to
        section 7.1.2 for use.
        Software that uses EINT shall clear it prior to clearing any IP flags. A race condition
        may occur if software clears the IP flags then clears the EINT flag, and between the
        operations another IP ‘0’ to '1' transition occurs. In this case the new IP transition
        shall be lost.
        When this register is exposed by a Virtual Function (VF), this bit is the logical 'OR' of
        the IP bits for the Interrupters assigned to the selected VF. And it shall be cleared to
        ‘0’ when all associated interrupter IP bits are cleared, that is, all the VF’s Interrupter
        Event Ring(s) are empty. Refer to section 8 for more information.
        */
        const EINT = 1 << 3;
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

#[derive(Debug, derive_mmio::Mmio)]
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

impl Display for MmioOperationalRegs<'_> {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "XHCI Operational Registers @{:?}:", VirtAddr::from_ptr(unsafe { self.ptr() }))?;
        writeln!(f, "\tusbcmd    : {:?}", self.read_usbcmd())?;
        writeln!(f, "\tusbstatus : {:?}", self.read_usbstatus())?;
        writeln!(f, "\tPage Size : {:#x}", self.read_page_size())?;
        writeln!(f, "\tdnctrl    : {:#x}", self.read_dnctrl())?;
        writeln!(f, "\tcrcr      : {:#x}", self.read_crcr())?;
        writeln!(f, "\tdcaap     : {:?}", self.read_dcbaap())?;
        write!(f,   "\tconfig    : {:#x}", self.read_config())?;
        Ok(())
    }
}

impl MmioOperationalRegs<'_> {
    pub unsafe fn port_registers(&mut self, port_index: u8) -> MmioPortRegisters<'static> {
        let ptr = unsafe { self.ptr() };
        unsafe {
            let port_reg_ptr = ptr
                .byte_add(0x400usize + (size_of::<PortRegisters>() * port_index as usize))
                as *mut PortRegisters;
            PortRegisters::new_mmio(port_reg_ptr)
        }
    }

    /// Reset a port at index `port_index`
    pub unsafe fn reset_port(&mut self, is_usb3: bool, port_index: u8) -> bool {
        let mut port_regs = unsafe { self.port_registers(port_index) };
        let mut port_sc = port_regs.read_port_sc();

        if !port_sc.pp() {
            // Power the port up
            port_regs.write_port_sc(port_sc.with_pp(true));

            // wait 20ms for power to stabilize
            sleep!(20 ms);

            port_sc = port_regs.read_port_sc();
            if !port_sc.pp() {
                warn!("xHCI port {} didn't power up, stopping reset", port_index);
                return false;
            }
        }

        // Clear any lingering status change bits before initiating the reset
        port_sc = port_regs
            .read_port_sc()
            .with_csc(true)
            .with_pec(true)
            .with_prc(true);

        port_regs.write_port_sc(port_sc);
        port_sc = port_regs.read_port_sc();

        if is_usb3 {
            // warm reset for usb3
            port_sc.set_wpr(true);
        } else {
            // standard port reset for usb2
            port_sc.set_pr(true);
        }

        port_regs.write_port_sc(port_sc);

        if !sleep_until!(
            100 ms,
            (!is_usb3 && port_regs.read_port_sc().prc()) || (is_usb3 && port_regs.read_port_sc().wrc())
        ) {
            warn!("xHCI port {port_index}: reset timeout after 100ms",);
            return false;
        }

        // wait 5ms for hardware to do it's thing
        sleep!(5 ms);

        port_sc = port_regs.read_port_sc();
        // Clear the reset completion and status change bits
        port_sc = port_sc
            /* clear port reset change */
            .with_prc(true)
            /* clear port warm reset change */
            .with_wrc(true)
            /* Clear connect status change */
            .with_csc(true)
            /* Clear port enable/disable change */
            .with_pec(true)
            /* leave port unenabled */
            .with_ped(false);
        port_regs.write_port_sc(port_sc);

        // wait 5ms for hardware to do it's thing
        sleep!(5 ms);

        // read to check if the port was reset successfully
        port_sc = port_regs.read_port_sc();

        // This case could happen when the port has been reset after
        // a device disconnect event, and no device has connected since.
        if !port_sc.ped() {
            warn!("xHCI attempted port {port_index} reset, port didn't enable, is_usb3 {is_usb3}");
            false
        } else {
            true
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortSpeed {
    Undefined = 0,
    /// 12 MB/s USB 2.0
    Full = 1,
    /// 1.5 Mb/s USB 2.0
    Low = 2,
    /// 480 Mb/s USB 2.0
    High = 3,
    /// 5 Gb/s (Gen1 x1) USB 3.0
    Super = 4,
    /// 10 Gb/s (Gen2 x1) USB 3.1
    SuperPlus = 5,
}

impl PortSpeed {
    pub const fn from_bits(bits: u8) -> Self {
        if bits > Self::SuperPlus as u8 {
            Self::Undefined
        } else {
            unsafe { core::mem::transmute(bits) }
        }
    }

    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// Returns the max initial packet size of a control transfer that has `self` port speed
    pub const fn max_control_transfer_initial_packet_size(&self) -> u16 {
        match self {
            Self::Low => 8,
            Self::Full | Self::High => 64,
            Self::Super | Self::SuperPlus => 512,
            Self::Undefined => 0,
        }
    }
}

/// Port Status & Control register
#[bitfield(u32)]
pub struct PortSCReg {
    /// Current Connect Status (CCS) – ROS. Default = ‘0’. ‘1’ = A device is connected81 to the port.
    /// ‘0’ =
    /// A device is not connected. This value reflects the current state of the port, and may not
    /// correspond directly to the event that caused the Connect Status Change (CSC) bit to be set to ‘1’.
    ///
    /// Refer to sections 4.19.3 and 4.19.4 for more details on the Connect Status Change (CSC)
    /// assertion conditions.
    ///
    /// This flag is ‘0’ if PP is ‘0’.
    pub ccs: bool,
    /// Port Enabled/Disabled (PED) – RW1CS. Default = ‘0’. ‘1’ = Enabled. ‘0’ = Disabled.
    /// Ports may only be enabled by the xHC. Software cannot enable a port by writing a ‘1’ to this flag.
    /// A port may be disabled by software writing a ‘1’ to this flag.
    /// This flag shall automatically be cleared to ‘0’ by a disconnect event or other fault condition.
    /// Note that the bit status does not change until the port state actually changes. There may be a
    /// delay in disabling or enabling a port due to other host controller or bus events.
    /// When the port is disabled (PED = ‘0’) downstream propagation of data is blocked on this port,
    /// except for reset.
    ///
    /// For USB2 protocol ports:
    ///
    /// When the port is in the Disabled state, software shall reset the port (PR = ‘1’) to transition PED to
    /// ‘1’ and the port to the Enabled state.
    ///
    /// For USB3 protocol ports:
    ///
    /// When the port is in the Polling state (after detecting an attach), the port shall automatically
    /// transition to the Enabled state and set PED to ‘1’ upon the completion of successful link training.
    /// When the port is in the Disabled state, software shall write a ‘5’ (RxDetect) to the PLS field to
    /// transition the port to the Disconnected state. Refer to section 4.19.1.2.
    /// PED shall automatically be cleared to ‘0’ when PR is set to ‘1’, and set to ‘1’ when PR transitions
    /// from ‘1’ to ‘0’ after a successful reset. Refer to Port Reset (PR) bit for more information on how
    /// the PED bit is managed.
    /// Note that when software writes this bit to a ‘1’, it shall also write a ‘0’ to the PR bit82.
    /// This flag is ‘0’ if PP is ‘0’.
    ped: bool,
    #[bits(2)]
    __: (),
    /// Port Reset (PR) – RW1S. Default = ‘0’. ‘1’ = Port Reset signaling is asserted. ‘0’ = Port is not in
    /// Rest. When software writes a ‘1’ to this bit generating a ‘0’ to ‘1’ transition, the bus reset
    /// sequence is initiated83; USB2 protocol ports shall execute the bus reset sequence as defined in
    /// the USB2 Spec. USB3 protocol ports shall execute the Hot Reset sequence as defined in the
    // USB3 Spec. PR remains set until reset signaling is completed by the root hub.
    /// Note that software shall write a ‘1’ to this flag to transition a USB2 port from the Polling state to
    /// the Enabled state. Refer to sections 4.15.2.3 and 4.19.1.1.
    /// This flag is ‘0’ if PP is ‘0’.
    pr: bool,
    #[bits(4)]
    __: (),
    /// Port Power (PP) – RWS. Default = ‘1’. This flag reflects a port's logical, power control state.
    /// Because host controllers can implement different methods of port power switching, this flag may
    /// or may not represent whether (VBus) power is actually applied to the port. When PP equals a '0'
    /// the port is nonfunctional and shall not report attaches, detaches, or Port Link State (PLS)
    /// changes. However, the port shall report over-current conditions when PP = ‘0’ if PPC = ‘0’. After
    /// modifying PP, software shall read PP and confirm that it is reached its target state before
    /// modifying it again91, undefined behavior may occur if this procedure is not followed.
    ///
    /// 0 = This port is in the Powered-off state.
    ///
    /// 1 = This port is not in the Powered-off state.
    ///
    /// If the Port Power Control (PPC) flag in the HCCPARAMS1 register is '1', then xHC has port power
    /// control switches and this bit represents the current setting of the switch ('0' = off, '1' = on).
    /// If the Port Power Control (PPC) flag in the HCCPARAMS1 register is '0', then xHC does not have
    /// port power control switches and each port is hard wired to power, and not affected by this bit.
    /// When an over-current condition is detected on a powered port, the xHC shall transition the PP
    /// bit in each affected port from a ‘1’ to ‘0’ (removing power from the port).
    /// Note: If this is an SSIC Port, then the DSP Disconnect process is initiated by '1' to '0' transition of
    /// PP. After an SSIC USP disconnect process, the port may be disabled by setting PED = 1. As noted,
    /// the SSIC spec does not define a mechanism for the USP to request DSP to be re-enabled for a
    /// subsequent re-connect. If PED is set to 1 without a prior negotiated disconnect with the USP,
    /// subsequent re-enabling of the port requires DSP to issue a WPR to bring USP back to Rx.Detect.
    /// Refer to section 5.1.2 in the SSIC Spec for more information.
    /// Refer to section 4.19.4 for more information.
    pp: bool,
    #[bits(4)]
    /// Speed (Port Speed) – ROS. Default = ‘0’. This field identifies the speed of the connected
    /// USB Device. This field is only relevant if a device is connected (CCS = ‘1’) in all other cases this
    /// field shall indicate Undefined Speed. Refer to section 4.19.3
    pub port_speed: PortSpeed,
    #[bits(3)]
    __: (),
    /// Connect Status Change (CSC) – RW1CS. Default = ‘0’. ‘1’ = Change in CCS. ‘0’ = No change.
    /// This flag indicates a change has occurred in the port’s Current Connect Status (CCS) or Cold Attach
    /// Status (CAS) bits. Note that this flag shall not be set if the CCS transition was due to software
    /// setting PP to ‘0’, or the CAS transition was due to software setting WPR to ‘1’. The xHC sets this
    /// bit to ‘1’ for all changes to the port device connect status92, even if system software has not
    /// cleared an existing Connect Status Change. For example, the insertion status changes twice
    /// before system software has cleared the changed condition, root hub hardware will be “setting”
    /// an already-set bit (i.e., the bit will remain ‘1’). Software shall clear this bit by writing a ‘1’ to it.
    /// Refer to section 4.19.2 for more information on change bit usage.
    pub csc: bool,
    /// Port Enabled/Disabled Change (PEC) – RW1CS. Default = ‘0’. ‘1’ = change in PED. ‘0’ = No
    /// change. Note that this flag shall not be set if the PED transition was due to software setting PP to
    /// ‘0’. Software shall clear this bit by writing a ‘1’ to it. Refer to section 4.19.2 for more information
    /// on change bit usage.
    /// For a USB2 protocol port, this bit shall be set to ‘1’ only when the port is disabled due to the
    /// appropriate conditions existing at the EOF2 point (refer to section 11.8.1 of the USB2
    /// Specification for the definition of a Port Error).
    /// For a USB3 protocol port, this bit shall never be set to ‘1’.
    pec: bool,
    //// Warm Port Reset Change (WRC) – RW1CS/RsvdZ. Default = ‘0’. This bit is set when Warm Reset
    /// processing on this port completes. ‘0’ = No change. ‘1’ = Warm Reset complete. Note that this
    /// flag shall not be set to ‘1’ if the Warm Reset processing was forced to terminate due to software
    /// clearing PP or PED to '0'. Software shall clear this bit by writing a '1' to it. Refer to section 4.19.5.1.
    /// Refer to section 4.19.2 for more information on change bit usage.
    /// This bit only applies to USB3 protocol ports. For USB2 protocol ports it shall be RsvdZ.
    wrc: bool,
    #[bits(1)]
    __: (),
    /// Port Reset Change (PRC) – RW1CS. Default = ‘0’. This flag is set to ‘1’ due to a '1' to '0' transition
    /// of Port Reset (PR). e.g. when any reset processing (Warm or Hot) on this port is complete. Note
    /// that this flag shall not be set to ‘1’ if the reset processing was forced to terminate due to software
    /// clearing PP or PED to '0'. ‘0’ = No change. ‘1’ = Reset complete. Software shall clear this bit by
    /// writing a '1' to it. Refer to section 4.19.5. Refer to section 4.19.2 for more information on change
    /// bit usage.
    prc: bool,
    #[bits(9)]
    __: (),
    /// Warm Port Reset (WPR) – RW1S/RsvdZ. Default = ‘0’. When software writes a ‘1’ to this bit, the
    /// Warm Reset sequence as defined in the USB3 Specification is initiated and the PR flag is set to ‘1’.
    /// Once initiated, the PR, PRC, and WRC flags shall reflect the progress of the Warm Reset
    /// sequence. This flag shall always return ‘0’ when read. Refer to section 4.19.5.1.
    /// This flag only applies to USB3 protocol ports. For USB2 protocol ports it shall be RsvdZ.
    wpr: bool,
}

#[derive(Debug, derive_mmio::Mmio)]
#[repr(C)]
pub struct PortRegisters {
    pub port_sc: PortSCReg,
    port_pmsc: u32,
    port_li: u32,
    __: u32,
}

const _: () = assert!(size_of::<PortRegisters>() == 0x10);

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct XHCIIman: u32 {
        /**
        # General Info
        - Interrupt Pending (IP)
        - RW1C
        - Default = ‘0’
        # Description
        > xHci Spec Section 5.5.2.1 Table 5-38: Interrupter Management Register Bit Definitions (IMAN) (page 425)

        This flag represents the current state of the Interrupter. If IP = ‘1’, an interrupt is pending for this Interrupter. A ‘0’ value indicates that no
        interrupt is pending for the Interrupter. Refer to section 4.17.3 for the conditions that modify
        the state of this flag.
        */
        const INTERRUPT_PENDING = 1 << 0;
        const INTERRUPT_ENABLE = 1 << 1;
    }
}

use bitfield_struct::bitfield;
#[bitfield(u64)]
pub struct EventRingDequePtr {
    #[bits(3)]
    pub erst_segment_index: usize,
    #[bits(1)]
    pub handler_busy: bool,
    #[bits(60)]
    pub _ptr_reset: u64,
}

impl EventRingDequePtr {
    pub const fn from_addr(addr: PhysAddr) -> Self {
        Self::from_bits(addr.into_raw() as u64)
    }

    pub const fn with_addr(self, addr: PhysAddr) -> Self {
        let bits = self.into_bits();
        let bits = bits | addr.into_raw() as u64;
        Self::from_bits(bits)
    }
}

#[derive(Debug, derive_mmio::Mmio)]
#[repr(C)]
pub struct InterrupterRegs {
    /// Interrupt management
    pub iman: XHCIIman,
    /// Interrupt moderation
    imod: u32,
    /// Event ring segment table size
    pub erst_sz: u32,
    __: u32,
    /// The base address of the event ring segment table
    pub erst_base: PhysAddr,
    pub event_ring_deque: EventRingDequePtr,
}

#[derive(Debug, derive_mmio::Mmio)]
#[repr(C)]
pub struct RuntimeRegs {
    /// Micro Frame index
    mf_index: u32,
    /// reserved
    __: [u32; 7],
    #[mmio(Inner)]
    interrupter_registers: [InterrupterRegs; 1024],
}

impl<'a> MmioRuntimeRegs<'a> {
    pub fn interrupter(&mut self, index: usize) -> MmioInterrupterRegs<'a> {
        unsafe {
            InterrupterRegs::new_mmio(
                self.interrupter_registers(index)
                    .expect("Interrupter out of bounds")
                    .ptr(),
            )
        }
    }
}

#[bitfield(u32)]
pub struct DoorbellReg {
    db_target: u8,
    __: u8,
    db_stream_id: u16,
}

#[derive(Debug, derive_mmio::Mmio)]
#[repr(C)]
pub struct DoorbellRegister {
    value: DoorbellReg,
}

#[derive(Debug)]
pub struct XHCIDoorbellManager<'a> {
    doorbells: *mut DoorbellRegister,
    max_device_slots: usize,
    _marker: core::marker::PhantomData<&'a mut DoorbellRegister>,
}

impl<'a> XHCIDoorbellManager<'a> {
    pub fn new(base: *mut DoorbellRegister, max_device_slots: usize) -> Self {
        Self {
            doorbells: base,
            max_device_slots,
            _marker: core::marker::PhantomData,
        }
    }

    pub fn ring_doorbell(&mut self, doorbell: u8, target: u8) {
        assert!((doorbell as usize) < self.max_device_slots);
        unsafe {
            let mut doorbell = DoorbellRegister::new_mmio(self.doorbells.add(doorbell as usize));
            let value = doorbell.read_value();
            doorbell.write_value(value.with_db_target(target));
        }
    }

    pub fn ring_command_doorbell(&mut self) {
        self.ring_doorbell(0, 0);
    }

    pub fn ring_control_endpoint_doorbell(&mut self, doorbell: u8) {
        self.ring_doorbell(doorbell, 1);
    }

    pub fn ring_endpoint_doorbell(&mut self, doorbell: u8, endpoint_num: u8) {
        self.ring_doorbell(doorbell, endpoint_num);
    }
}

#[derive(Debug)]
/// A general wrapper around XHCI's registers such as capabilities, operationals, and runtime
pub struct XHCIRegisters<'s> {
    caps_regs: MmioCapsReg<'s>,
    op_regs: MmioOperationalRegs<'s>,
    runtime_regs: MmioRuntimeRegs<'s>,
    // TODO: free the frames when this goes out of scope? except that currently it never does
    /// used to store the scratchpad_buffers pointers and the dcbaa (scratchpad_buffers, dcbaa)
    buffers_frame: Frame,
    scratchpad_buffers: Option<&'s mut [Frame]>,
    dcbaa: &'s mut [PhysAddr],
}

impl<'s> XHCIRegisters<'s> {
    /// Creates a new XHCI Register manager that owns the XHCI Registers area
    /// resets the XHCI controller to zero status
    /// unsafe because it asseums ownership of the XHCI registers
    pub unsafe fn new(caps: MmioCapsReg<'s>) -> Self {
        unsafe {
            let mut this = Self {
                op_regs: caps.operational_regs(),
                runtime_regs: caps.runtime_regs(),
                caps_regs: caps,
                buffers_frame: frame_allocator::allocate_frame()
                    .expect("failed to allocate frame for the XHCI buffers"),
                scratchpad_buffers: None,
                dcbaa: &mut [],
            };
            this.reset_zero();
            this
        }
    }

    pub unsafe fn capabilities<'sel>(&'sel self) -> MmioCapsReg<'sel> {
        unsafe { self.caps_regs.clone() }
    }

    pub unsafe fn operational_regs(&mut self) -> &mut MmioOperationalRegs<'s> {
        &mut self.op_regs
    }

    unsafe fn runtime_regs(&mut self) -> &mut MmioRuntimeRegs<'s> {
        &mut self.runtime_regs
    }

    pub unsafe fn set_dcbaa_entry(&mut self, slot_id: u8, entry: PhysAddr) {
        let slot_id = slot_id as usize;
        assert!(slot_id < self.dcbaa.len());
        assert!(slot_id != 0);

        let ptr = self.dcbaa.as_mut_ptr();
        unsafe {
            ptr.add(slot_id).write_volatile(entry);
        }
    }

    pub unsafe fn get_dcbaa_entry_as_ptr(&mut self, slot_id: u8) -> *mut XHCIDeviceCtx32 {
        let slot_id = slot_id as usize;
        assert!(slot_id < self.dcbaa.len());
        assert!(slot_id != 0);
        let ptr = self.dcbaa.as_mut_ptr();
        unsafe { ptr.add(slot_id).read_volatile().into_virt().into_ptr() }
    }

    /// Clear any incoming interrupts for the interrupter
    pub unsafe fn acknowledge_irq(&mut self, interrupter: u8) {
        let op_regs = unsafe { self.operational_regs() };
        // Write the USBSts::EINT bit to clear it, it is RW1C meaning write 1 to clear
        op_regs.write_usbstatus(USBSts::EINT);

        let runtime_regs = unsafe { self.runtime_regs() };
        let mut interrupt_reg = runtime_regs.interrupter(interrupter as usize);
        // Similariy we clear the iman interrupt pending bit by writing 1 to it
        let iman = interrupt_reg.read_iman() | XHCIIman::INTERRUPT_PENDING;
        interrupt_reg.write_iman(iman);
    }

    /// Starts the XHCI controller
    pub unsafe fn start(&mut self) {
        let regs = unsafe { self.operational_regs() };
        regs.write_usbcmd(regs.read_usbcmd() | USBCmd::RUN | USBCmd::INTERRUPT_ENABLE);

        if !sleep_until!(1000 ms, !regs.read_usbstatus().contains(USBSts::HCHALTED)) {
            panic!(
                "timeout after 1 second while resetting the XHCI, HCHALTED did not clear: {:?}",
                regs.read_usbstatus()
            )
        }

        assert!(!regs.read_usbstatus().contains(USBSts::NOT_READY));
    }

    #[allow(unused_unsafe)]
    /// Resets the XHCI controller to zero status
    /// Unsafe because the controller needs to be reconfigured after this
    pub unsafe fn reset_zero(&mut self) {
        unsafe {
            let regs = self.operational_regs();

            regs.write_usbcmd(regs.read_usbcmd() & !USBCmd::RUN);

            if !sleep_until!(200 ms, regs.read_usbstatus().contains(USBSts::HCHALTED)) {
                panic!(
                    "timeout after 200ms while resetting the XHCI, HCHALTED did not set: {:?}",
                    regs.read_usbstatus()
                )
            }

            // reset the controller
            regs.write_usbcmd(regs.read_usbcmd() | USBCmd::HCRESET);

            if !sleep_until!(1000 ms,
                !regs.read_usbcmd().contains(USBCmd::HCRESET)
                                && !regs.read_usbstatus().contains(USBSts::NOT_READY)
            ) {
                panic!(
                    "timeout after 1000ms while resetting controller, controller was never ready: {:?}",
                    regs.read_usbcmd(),
                )
            }
            // asserts the controller was reset
            assert_eq!(regs.read_usbcmd(), USBCmd::empty());
            assert_eq!(regs.read_dnctrl(), 0);
            assert_eq!(regs.read_crcr(), 0);
            assert_eq!(regs.read_dcbaap(), PhysAddr::null());
            assert_eq!(regs.read_config(), 0);
            debug!(XHCIRegisters, "XHCI Reset\n{}", regs,);
        }
    }

    /// Reconfigures the XHCI controller given an event ring and a command ring
    pub unsafe fn reconfigure(
        &mut self,
        event_ring: &mut XHCIEventRing,
        command_ring: &XHCICommandRing,
    ) {
        let max_device_slots = unsafe { self.capabilities() }.max_device_slots() as u32;
        let op_regs = unsafe { self.operational_regs() };
        op_regs.write_config(max_device_slots);
        // Enable device notifications
        op_regs.write_dnctrl(0xFFFF);
        self.configure_dcbaa();
        self.configure_crcr(command_ring);

        self.configure_runtime(event_ring);
    }

    fn configure_crcr(&mut self, command_ring: &XHCICommandRing) {
        let op_regs = unsafe { self.operational_regs() };
        op_regs.write_crcr(
            *command_ring.base_phys_addr() | command_ring.current_ring_cycle() as usize,
        );
    }

    fn configure_dcbaa(&mut self) {
        let caps = unsafe { self.capabilities() };
        let max_device_slots = caps.max_device_slots();
        let max_scratchpad_buffers = caps.max_scratchpad_buffers();

        // Allocates and sets the dcbaa
        assert!(max_device_slots * size_of::<PhysAddr>() <= PAGE_SIZE);

        let (dcbaa_slice, dcbaa_phys_addr) =
            allocate_buffers_frame::<PhysAddr>(self.buffers_frame, 0, max_device_slots);

        // Allocates the scratchpad buffers array if neccassary
        if max_scratchpad_buffers > 0 {
            // uses the same frame to store the scratchpad_buffers pointers that we used to store dcbaa entries
            // it is safe to do so as the max number of dcbaa entries is 255,
            // and the max numbers of scratchpad_buffers is 15, (255 + 15) * 8 is very much less then the maximum amount of bytes a frame (page) can hold (4096)
            // DCBAA entries must be 64 byte aligned
            let (scratchpad_buffers, scratchpad_buffers_addr) = allocate_buffers_frame::<Frame>(
                self.buffers_frame,
                (dcbaa_phys_addr + dcbaa_slice.len())
                    .to_next_multiple_of(64)
                    .into_raw(),
                max_scratchpad_buffers,
            );

            for phys_addr in scratchpad_buffers.iter_mut() {
                *phys_addr = frame_allocator::allocate_frame()
                    .expect("XHCI: failed to allocate a page for a scratchpad buffer");
            }
            self.scratchpad_buffers = Some(scratchpad_buffers);
            // DCBAA[0] is used to store the address of the scratchpad_buffers
            self.dcbaa[0] = scratchpad_buffers_addr;
        }

        self.dcbaa = dcbaa_slice;
        let op_regs = unsafe { self.operational_regs() };
        op_regs.write_dcbaap(dcbaa_phys_addr);
    }

    fn configure_runtime(&mut self, event_ring: &mut XHCIEventRing) {
        event_ring.reset();
        let runtime_regs = unsafe { self.runtime_regs() };
        let mut interrupt_reg = runtime_regs.interrupter(0);
        // Enable interrupts
        interrupt_reg.write_iman(XHCIIman::INTERRUPT_ENABLE);

        // Clear any pending interrupts
        unsafe {
            self.acknowledge_irq(0);
        }
    }
}
