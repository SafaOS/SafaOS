use crate::{
    debug,
    drivers::{
        utils::{read_ref, write_ref},
        xhci::{
            rings::{XHCICommandRing, XHCIEventRing},
            utils::allocate_buffers_frame,
        },
    },
    memory::{
        align_up,
        frame_allocator::{self, Frame},
        paging::PAGE_SIZE,
    },
    time, PhysAddr, VirtAddr,
};
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
    doorbell_off: u32,
    runtime_off: u32,
    hccparams_2: u32,
}

impl CapsReg {
    pub fn operational_regs_mut<'a>(&mut self) -> &'a mut OperationalRegs {
        let caps_ptr = self as *const _ as *const u8;
        unsafe {
            let ptr = caps_ptr.add(self.reg_length as usize);
            &mut *(ptr as *mut OperationalRegs)
        }
    }

    pub fn runtime_regs_mut<'a>(&mut self) -> &'a mut RuntimeRegs {
        let caps_ptr = self as *const _ as *const u8;
        unsafe {
            let ptr = caps_ptr.add(self.runtime_off as usize);
            &mut *(ptr as *mut RuntimeRegs)
        }
    }

    pub fn doorbells_base(&mut self) -> VirtAddr {
        let caps_ptr = self as *const _ as *const u8;
        unsafe {
            let ptr = caps_ptr.add(self.doorbell_off as usize);
            let addr = VirtAddr::from_ptr(ptr);
            addr
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

#[derive(Debug)]
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

#[repr(C)]
pub struct RuntimeRegs {
    /// Micro Frame index
    mf_index: u32,
    /// reserved
    __: [u32; 7],
    interrupter_registers: [InterrupterRegs; 1024],
}

impl RuntimeRegs {
    pub fn interrupter_mut(&mut self, index: usize) -> &mut InterrupterRegs {
        &mut self.interrupter_registers[index]
    }
}

#[bitfield(u32)]
pub struct DoorbellReg {
    db_target: u8,
    __: u8,
    db_stream_id: u16,
}

#[derive(Debug)]
pub struct XHCIDoorbellManager<'a> {
    doorbells: &'a mut [DoorbellReg],
}

impl<'a> XHCIDoorbellManager<'a> {
    pub fn new(base: VirtAddr, max_device_slots: usize) -> Self {
        let doorbells_ptr = base.into_ptr::<DoorbellReg>();
        let doorbells = unsafe { core::slice::from_raw_parts_mut(doorbells_ptr, max_device_slots) };
        Self { doorbells }
    }

    pub fn ring_doorbell(&mut self, doorbell: u8, target: u8) {
        self.doorbells[doorbell as usize].set_db_target(target);
    }

    pub fn ring_command_doorbell(&mut self) {
        self.ring_doorbell(0, 0);
    }

    pub fn ring_control_endpoint_doorbell(&mut self, doorbell: u8) {
        self.ring_doorbell(doorbell, 1);
    }
}

#[derive(Debug)]
/// A general wrapper around XHCI's registers such as captabilities, operationals, and runtime
pub struct XHCIRegisters<'s> {
    caps_regs: *mut CapsReg,
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
    pub unsafe fn new(caps: *mut CapsReg) -> Self {
        let mut this = Self {
            caps_regs: caps,
            buffers_frame: frame_allocator::allocate_frame()
                .expect("failed to allocate frame for the XHCI buffers"),
            scratchpad_buffers: None,
            dcbaa: &mut [],
        };
        unsafe {
            this.reset_zero();
        }
        this
    }

    pub fn captabilities<'a>(&self) -> &'a CapsReg {
        unsafe { &*self.caps_regs }
    }

    fn captabilities_mut<'a>(&mut self) -> &'a mut CapsReg {
        unsafe { &mut *self.caps_regs }
    }

    pub fn operational_regs<'a>(&mut self) -> &'a mut OperationalRegs {
        let caps = self.captabilities_mut();
        caps.operational_regs_mut()
    }

    fn runtime_regs<'a>(&mut self) -> &'a mut RuntimeRegs {
        self.captabilities_mut().runtime_regs_mut()
    }

    /// Clear any incoming interrupts for the interrupter
    pub fn acknowledge_irq(&mut self, interrupter: u8) {
        let op_regs = self.operational_regs();
        // Write the USBSts::EINT bit to clear it, it is RW1C meaning write 1 to clear
        write_ref!(op_regs.usbstatus, USBSts::EINT);

        let runtime_regs = self.runtime_regs();
        let interrupt_reg = runtime_regs.interrupter_mut(interrupter as usize);
        // Similariy we clear the iman interrupt pending bit by writing 1 to it
        let iman = interrupt_reg.iman | XHCIIman::INTERRUPT_PENDING;
        write_ref!(interrupt_reg.iman, iman);
    }

    /// Starts the XHCI controller
    pub unsafe fn start(&mut self) {
        let regs = self.operational_regs();
        write_ref!(
            regs.usbcmd,
            regs.usbcmd | USBCmd::RUN | USBCmd::INTERRUPT_ENABLE
        );

        let timeout = 1000;
        let time = time!();

        while read_ref!(regs.usbstatus).contains(USBSts::HCHALTED) {
            let now = time!();
            if now >= time + timeout {
                panic!(
                    "timeout after {}ms while resetting the XHCI, HCHALTED did not clear: {:?}",
                    now,
                    read_ref!(regs.usbstatus)
                )
            }
            core::hint::spin_loop();
        }

        assert!(!read_ref!(regs.usbstatus).contains(USBSts::NOT_READY));
    }

    #[allow(unused_unsafe)]
    /// Resets the XHCI controller to zero status
    /// Unsafe because the controller needs to be reconfigured after this
    pub unsafe fn reset_zero(&mut self) {
        let regs = self.operational_regs();

        write_ref!(regs.usbcmd, regs.usbcmd & !USBCmd::RUN);

        let timeout = 200;
        let time = time!();

        while !read_ref!(regs.usbstatus).contains(USBSts::HCHALTED) {
            let now = time!();
            if now >= time + timeout {
                panic!(
                    "timeout after {}ms while resetting the XHCI, HCHALTED did not set: {:?}",
                    now,
                    read_ref!(regs.usbstatus)
                )
            }
            core::hint::spin_loop();
        }

        // reset the controller
        write_ref!(regs.usbcmd, read_ref!(regs.usbcmd) | USBCmd::HCRESET);

        let timeout = 1000;
        let time = time!();

        while read_ref!(regs.usbcmd).contains(USBCmd::HCRESET)
            || read_ref!(regs.usbstatus).contains(USBSts::NOT_READY)
        {
            let now = time!();
            if now >= time + timeout {
                panic!(
                    "timeout after {}ms while resetting controller, controller was never ready: {:?}",
                    now - time,
                    read_ref!(regs.usbcmd),
                )
            }
            core::hint::spin_loop();
        }
        // asserts the controller was reset
        assert_eq!(regs.usbcmd, USBCmd::empty());
        assert_eq!(regs.dnctrl, 0);
        assert_eq!(regs.crcr, 0);
        assert_eq!(regs.dcbaap, PhysAddr::null());
        assert_eq!(regs.config, 0);
        debug!(XHCIRegisters, "XHCI Reset\n{}", regs,);
    }

    /// Reconfigures the XHCI controller given an event ring and a command ring
    pub unsafe fn reconfigure(
        &mut self,
        event_ring: &mut XHCIEventRing,
        command_ring: &XHCICommandRing,
    ) {
        let op_regs = self.operational_regs();
        write_ref!(
            op_regs.config,
            self.captabilities().max_device_slots() as u32
        );
        // Enable device notifications
        write_ref!(op_regs.dnctrl, 0xFFFF);
        self.configure_dcbaa();
        self.configure_crcr(command_ring);

        self.configure_runtime(event_ring);
    }

    fn configure_crcr(&mut self, command_ring: &XHCICommandRing) {
        let op_regs = self.operational_regs();
        write_ref!(
            op_regs.crcr,
            *command_ring.base_phys_addr() | command_ring.current_ring_cycle() as usize
        );
    }

    fn configure_dcbaa(&mut self) {
        let caps = self.captabilities();
        let op_regs = self.operational_regs();

        // Allocates and sets the dcbaa
        assert!(caps.max_device_slots() * size_of::<PhysAddr>() <= PAGE_SIZE);

        let (dcbaa_slice, dcbaa_phys_addr) =
            allocate_buffers_frame::<PhysAddr>(self.buffers_frame, 0, caps.max_device_slots());

        // Allocates the scratchpad buffers array if neccassary
        if caps.max_scratchpad_buffers() > 0 {
            // uses the same frame to store the scratchpad_buffers pointers that we used to store dcbaa entries
            // it is safe to do so as the max number of dcbaa entries is 255,
            // and the max numbers of scratchpad_buffers is 15, (255 + 15) * 8 is very much less then the maximum amount of bytes a frame (page) can hold (4096)
            // DCBAA entries must be 64 byte aligned
            let (scratchpad_buffers, scratchpad_buffers_addr) = allocate_buffers_frame::<Frame>(
                self.buffers_frame,
                align_up((dcbaa_phys_addr + dcbaa_slice.len()).into_raw(), 64),
                caps.max_scratchpad_buffers(),
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
        write_ref!(op_regs.dcbaap, dcbaa_phys_addr);
    }

    fn configure_runtime(&mut self, event_ring: &mut XHCIEventRing) {
        event_ring.reset();
        let runtime_regs = self.runtime_regs();
        let interrupt_reg = runtime_regs.interrupter_mut(0);
        // Enable interrupts
        write_ref!(interrupt_reg.iman, XHCIIman::INTERRUPT_ENABLE);

        // Clear any pending interrupts
        self.acknowledge_irq(0);
    }
}
