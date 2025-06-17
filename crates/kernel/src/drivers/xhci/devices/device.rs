use crate::{
    debug,
    drivers::xhci::{
        self,
        devices::{
            DeviceEndpointState, DeviceEndpointType, XHCIEndpointDeviceCtx32,
            XHCIInputControlCtx32, XHCIInputCtx32, XHCIInputCtx64, XHCISlotDeviceCtx32,
        },
        regs::PortSpeed,
        rings::{
            transfer::XHCITransferRing,
            trbs::{PacketRecipient, PacketType, XHCIDeviceRequestPacket},
        },
        usb::UsbDeviceDescriptor,
        utils::XHCIError,
        XHCIResponseQueue, MAX_TRB_COUNT,
    },
    PhysAddr,
};

pub const REQUEST_GET_DESCRIPTOR: u8 = 6;

#[derive(Debug, Clone, Copy)]
enum InputCtxPtr {
    Size64(*mut XHCIInputCtx64),
    Size32(*mut XHCIInputCtx32),
}

pub struct XHCIDevice {
    input_ctx_ptr: InputCtxPtr,
    input_ctx_base_addr: PhysAddr,

    xhci_transfer_ring: XHCITransferRing,

    port_index: u8,
    port_speed: PortSpeed,
    slot_id: u8,
}

impl XHCIDevice {
    pub const fn transfer_ring(&mut self) -> &mut XHCITransferRing {
        &mut self.xhci_transfer_ring
    }

    pub const fn input_ctx_base_addr(&self) -> PhysAddr {
        self.input_ctx_base_addr
    }

    pub const fn port_id(&self) -> u8 {
        self.port_index + 1
    }

    pub const fn slot_id(&self) -> u8 {
        self.slot_id
    }

    fn get_input_ctrl_ctx(&mut self) -> *mut XHCIInputControlCtx32 {
        unsafe {
            match self.input_ctx_ptr {
                InputCtxPtr::Size64(ctx) => (&raw mut (*ctx).input_control_context).cast(),
                InputCtxPtr::Size32(ctx) => (&raw mut (*ctx).input_control_context),
            }
        }
    }

    pub fn get_slot_ctx(&mut self) -> *mut XHCISlotDeviceCtx32 {
        unsafe {
            match self.input_ctx_ptr {
                InputCtxPtr::Size64(ctx) => (&raw mut (*ctx).device_context.slot_context).cast(),
                InputCtxPtr::Size32(ctx) => (&raw mut (*ctx).device_context.slot_context),
            }
        }
    }

    pub fn get_ctrl_endpoint_ctx(&mut self) -> *mut XHCIEndpointDeviceCtx32 {
        unsafe {
            match self.input_ctx_ptr {
                InputCtxPtr::Size64(ctx) => {
                    (&raw mut (*ctx).device_context.control_ep_context).cast()
                }
                InputCtxPtr::Size32(ctx) => (&raw mut (*ctx).device_context.control_ep_context),
            }
        }
    }

    pub fn create(
        use_64byte_ctx: bool,
        port_index: u8,
        slot_id: u8,
        port_speed: PortSpeed,
    ) -> Self {
        let input_ctx_sz = if use_64byte_ctx {
            size_of::<XHCIInputCtx64>()
        } else {
            size_of::<XHCIInputCtx32>()
        };

        let (input_ctx_bytes, input_ctx_base_addr) = xhci::utils::allocate_buffers(input_ctx_sz)
            .expect("failed to allocate memory for an XHCI Device's input context");

        let input_ctx_ptr_raw: *mut u8 = input_ctx_bytes.as_mut_ptr();
        let input_ctx_ptr = if use_64byte_ctx {
            InputCtxPtr::Size64(input_ctx_ptr_raw.cast())
        } else {
            InputCtxPtr::Size32(input_ctx_ptr_raw.cast())
        };

        Self {
            input_ctx_ptr,
            input_ctx_base_addr,
            xhci_transfer_ring: XHCITransferRing::create(MAX_TRB_COUNT, slot_id),
            port_index,
            slot_id,
            port_speed,
        }
    }

    pub fn configure_ctrl_ep_input_ctx(&mut self, max_packet_size: u16) {
        let in_control_ctx = unsafe { &mut *self.get_input_ctrl_ctx() };
        let slot_ctx = unsafe { &mut *self.get_slot_ctx() };
        let endpoint_ctx = unsafe { &mut *self.get_ctrl_endpoint_ctx() };
        // Enable slot and control endpoint contexts
        in_control_ctx.add_ctx_flags = (1 << 0) | (1 << 1);
        in_control_ctx.drop_flags = 0;

        // Configure the slot context
        slot_ctx.dword0.set_context_entries(1);
        slot_ctx.dword0.set_speed(self.port_speed);
        slot_ctx.dword0.set_route_string(0);
        // TODO: all ports for now are treated as root hubs
        slot_ctx.dword1.set_root_hub_port_id(self.port_id());
        slot_ctx.dword2.set_parent_port_id(0);
        // TODO: we only use interrupter 0 for now
        slot_ctx.dword2.set_interrupter_target(0);

        // Configure the control endpoint
        endpoint_ctx
            .dword0
            .set_endpoint_state(DeviceEndpointState::Disabled);
        endpoint_ctx.dword0.set_max_esit_payload_hi(0);
        endpoint_ctx.dword0.set_interval(0);
        endpoint_ctx.dword1.set_err_cnt(3);
        endpoint_ctx
            .dword1
            .set_er_type(DeviceEndpointType::ControlBI);
        endpoint_ctx.dword1.set_max_packet_size(max_packet_size);
        endpoint_ctx.qword2.set_trb_dequeue_ptr(
            self.xhci_transfer_ring.get_physical_dequeue_pointer_base(),
            self.xhci_transfer_ring.curr_ring_cycle_bit(),
        );

        endpoint_ctx.max_esit_payload_low = 0;
        endpoint_ctx.average_trb_length = 8;
        debug!(XHCIDevice, "configured cntrl endpoint for device with slot {} and port {}, set max packet size to {max_packet_size}", self.slot_id(), self.port_id());
    }

    pub fn fill_usb_descriptor(
        &mut self,
        xhci_queue_manager: &XHCIResponseQueue,
        descriptor: &mut UsbDeviceDescriptor,
        len: usize,
    ) -> Result<(), XHCIError> {
        let packet = XHCIDeviceRequestPacket::new()
            .with_p_type(PacketType::Standard)
            .with_recipient(PacketRecipient::Device)
            .with_device_to_host(true)
            .with_w_length(len as u16)
            // GET_DESCRIPTOR
            .with_b_request(REQUEST_GET_DESCRIPTOR)
            .with_w_index(0)
            // Low byte = 0 (Descriptor Index), High Byte = 1
            // (Descriptor type).
            .with_w_value(0x100);

        let output_bytes: &mut [u8; size_of::<UsbDeviceDescriptor>()] =
            unsafe { core::mem::transmute(descriptor) };
        xhci_queue_manager.send_request_packet(self, packet, &mut output_bytes[..len])
    }
}
