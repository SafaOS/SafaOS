use alloc::vec::Vec;

use crate::{
    debug,
    drivers::xhci::{
        self,
        devices::{
            DeviceEndpointState, DeviceEndpointType, XHCIDeviceCtx32, XHCIEndpointDeviceCtx32,
            XHCIInputControlCtx32, XHCIInputCtx32, XHCIInputCtx64, XHCISlotDeviceCtx32,
        },
        regs::PortSpeed,
        rings::{
            transfer::XHCITransferRing,
            trbs::{PacketRecipient, PacketType, XHCIDeviceRequestPacket},
        },
        usb::{
            UsbConfigurationDescriptor, UsbDescriptorHeader, UsbDeviceDescriptor,
            UsbEndpointDescriptor, USB_DESCRIPTOR_CONFIGURATION_TYPE, USB_DESCRIPTOR_DEVICE_TYPE,
        },
        utils::XHCIError,
        XHCIResponseQueue, MAX_TRB_COUNT,
    },
    error, write_ref, PhysAddr,
};

pub const REQUEST_GET_DESCRIPTOR: u8 = 6;
pub const REQUEST_SET_CONFIGURATION: u8 = 9;

#[derive(Debug, Clone, Copy)]
enum InputCtxPtr {
    Size64(*mut XHCIInputCtx64),
    Size32(*mut XHCIInputCtx32),
}

#[derive(Debug)]
pub struct XHCIDevice {
    input_ctx_ptr: InputCtxPtr,
    input_ctx_base_addr: PhysAddr,

    xhci_transfer_ring: XHCITransferRing,
    endpoints: Vec<(UsbEndpointDescriptor, XHCITransferRing)>,

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

    pub unsafe fn get_input_device_ctx(&mut self) -> *mut XHCIDeviceCtx32 {
        unsafe {
            match self.input_ctx_ptr {
                InputCtxPtr::Size64(ctx) => (&raw mut (*ctx).device_context).cast(),
                InputCtxPtr::Size32(ctx) => (&raw mut (*ctx).device_context),
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

    pub fn get_input_endpoint_ctx(&mut self, endpoint_num: u8) -> *mut XHCIEndpointDeviceCtx32 {
        let endpoint_num = endpoint_num as usize - 2;
        unsafe {
            match self.input_ctx_ptr {
                InputCtxPtr::Size64(ctx) => {
                    (&raw mut (*ctx).device_context.ep[endpoint_num]).cast()
                }
                InputCtxPtr::Size32(ctx) => (&raw mut (*ctx).device_context.ep[endpoint_num]),
            }
        }
    }

    pub fn create(
        use_64byte_ctx: bool,
        port_index: u8,
        slot_id: u8,
        port_speed: PortSpeed,
    ) -> Result<Self, XHCIError> {
        let input_ctx_sz = if use_64byte_ctx {
            size_of::<XHCIInputCtx64>()
        } else {
            size_of::<XHCIInputCtx32>()
        };

        let (input_ctx_bytes, input_ctx_base_addr) =
            xhci::utils::allocate_buffers(input_ctx_sz).ok_or(XHCIError::OutOfMemory)?;

        let input_ctx_ptr_raw: *mut u8 = input_ctx_bytes.as_mut_ptr();
        let input_ctx_ptr = if use_64byte_ctx {
            InputCtxPtr::Size64(input_ctx_ptr_raw.cast())
        } else {
            InputCtxPtr::Size32(input_ctx_ptr_raw.cast())
        };

        Ok(Self {
            input_ctx_ptr,
            input_ctx_base_addr,
            xhci_transfer_ring: XHCITransferRing::create(MAX_TRB_COUNT, slot_id)?,
            endpoints: Vec::new(),
            port_index,
            slot_id,
            port_speed,
        })
    }

    /// Configures the endpoint with USB Descriptor `endpoint` and the transfer ring `endpoint_transfer_ring`
    /// Unsafe because this should only called once per endpoint
    pub unsafe fn configure_ep_input_ctx(
        &mut self,
        endpoint: UsbEndpointDescriptor,
    ) -> Result<(), XHCIError> {
        let in_control_ctx = unsafe { &mut *self.get_input_ctrl_ctx() };
        let slot_ctx = unsafe { &mut *self.get_slot_ctx() };

        let transfer_ring = XHCITransferRing::create(MAX_TRB_COUNT, self.slot_id())?;
        let endpoint_num = endpoint.endpoint_num();
        let endpoint_type = endpoint.endpoint_type();

        in_control_ctx.add_ctx_flags |= 1 << endpoint_num;
        in_control_ctx.drop_flags = 0;
        crate::serial!(
            "{endpoint_num}, {endpoint_type:#?}: {}\n",
            slot_ctx.dword0.context_entries()
        );
        if endpoint_num > slot_ctx.dword0.context_entries() {
            slot_ctx.dword0.set_context_entries(endpoint_num);
        }

        let endpoint_ctx = unsafe { &mut *self.get_input_endpoint_ctx(endpoint_num) };
        write_ref!(
            endpoint_ctx.dword0,
            endpoint_ctx
                .dword0
                .with_endpoint_state(DeviceEndpointState::Disabled)
        );
        write_ref!(
            endpoint_ctx.dword1,
            endpoint_ctx
                .dword1
                .with_max_packet_size(endpoint.max_packet_size())
                .with_er_type(endpoint_type)
                .with_max_brust_size(0)
                .with_err_cnt(3)
        );
        write_ref!(endpoint_ctx.average_trb_length, endpoint.max_packet_size());
        write_ref!(endpoint_ctx.average_trb_length, endpoint.max_packet_size());
        write_ref!(
            endpoint_ctx.qword2,
            endpoint_ctx.qword2.with_trb_dequeue_ptr(
                transfer_ring.get_physical_dequeue_pointer_base(),
                transfer_ring.curr_ring_cycle_bit(),
            )
        );

        if self.port_speed == PortSpeed::High || self.port_speed == PortSpeed::Super {
            let interval = endpoint.b_interval - 1;
            write_ref!(
                endpoint_ctx.dword0,
                endpoint_ctx.dword0.with_interval(interval)
            );
        } else {
            todo!("endpoint intervals for speed {:?}", self.port_speed)
        }

        self.endpoints.push((endpoint, transfer_ring));
        Ok(())
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

    /// Disables the control endpoint
    pub fn disable_ctrl_endpoint(&mut self) {
        let in_control_ctx = unsafe { &mut *self.get_input_ctrl_ctx() };
        in_control_ctx.add_ctx_flags = 1 << 0;
        in_control_ctx.drop_flags = 0;
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
            .with_w_value((USB_DESCRIPTOR_DEVICE_TYPE << 8) | (0));

        let output_bytes: &mut [u8; size_of::<UsbDeviceDescriptor>()] =
            unsafe { core::mem::transmute(descriptor) };
        xhci_queue_manager.send_request_packet(self, packet, &mut output_bytes[..len])
    }

    pub fn get_usb_configuration_descriptor(
        &mut self,
        xhci_queue_manager: &XHCIResponseQueue,
    ) -> Result<UsbConfigurationDescriptor, XHCIError> {
        let mut bytes_raw: [u8; size_of::<UsbConfigurationDescriptor>()] =
            [0; size_of::<UsbConfigurationDescriptor>()];
        let bytes_ptr = &raw mut bytes_raw;
        let ptr = bytes_ptr as *mut UsbConfigurationDescriptor;

        let mut packet = XHCIDeviceRequestPacket::new()
            .with_p_type(PacketType::Standard)
            .with_recipient(PacketRecipient::Device)
            .with_device_to_host(true)
            .with_w_length(size_of::<UsbDescriptorHeader>() as u16)
            // GET_DESCRIPTOR
            .with_b_request(REQUEST_GET_DESCRIPTOR)
            .with_w_index(0)
            // Low byte = 0 (Descriptor Index), High Byte = 2
            // (Descriptor type).
            .with_w_value((USB_DESCRIPTOR_CONFIGURATION_TYPE << 8) | 0);

        unsafe {
            // First read just the header in order to get the total descriptor size
            xhci_queue_manager.send_request_packet(
                self,
                packet,
                &mut (&mut *bytes_ptr)[..size_of::<UsbDescriptorHeader>()],
            )?;
            // read the entire descriptor
            let header_len = (*ptr).header.b_length as usize;
            packet.set_w_length(header_len as u16);
            xhci_queue_manager.send_request_packet(
                self,
                packet,
                &mut (&mut *bytes_ptr)[..header_len],
            )?;

            // Now we get the total bytes
            // read the additional bytes for interface descriptors as well
            let total_len = (*ptr).w_total_len as usize;
            if total_len > size_of::<UsbConfigurationDescriptor>() - 1 {
                error!(XHCIDevice, "USB Configuration descriptor size {total_len} is more then the supported size {}", size_of::<UsbConfigurationDescriptor>() - 1);
                return Err(XHCIError::Other);
            }

            packet.set_w_length(total_len as u16);
            xhci_queue_manager.send_request_packet(
                self,
                packet,
                &mut (&mut *bytes_ptr)[..total_len],
            )?;

            Ok(core::mem::transmute(bytes_raw))
        }
    }

    pub fn set_configuration(
        &mut self,
        xhci_queue_manager: &XHCIResponseQueue,
        configuration: u16,
    ) -> Result<(), XHCIError> {
        let packet = XHCIDeviceRequestPacket::new()
            .with_p_type(PacketType::Standard)
            .with_recipient(PacketRecipient::Device)
            .with_device_to_host(false)
            .with_b_request(REQUEST_SET_CONFIGURATION)
            .with_w_index(0)
            .with_w_length(0)
            .with_w_value(configuration);
        xhci_queue_manager.send_no_data_request_packet(self, packet)
    }
}
