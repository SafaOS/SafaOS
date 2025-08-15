use core::cell::SyncUnsafeCell;

use bitfield_struct::bitfield;
use safa_abi::input::{MiceBtnStatus, MiceEvent, MouseEventKind};

use crate::{
    arch::x86_64::{inb, outb},
    devices::input,
    info,
};

pub const PS2_DATA_PORT: u16 = 0x60;
pub const PS2_COMMAND_PORT: u16 = 0x64;
pub const PS2_STATUS_PORT: u16 = PS2_COMMAND_PORT;

#[bitfield(u8)]
struct MicePacketHeader {
    btn_left: bool,
    btn_right: bool,
    btn_middle: bool,

    always_true: bool,

    x_axis_neg: bool,
    y_axis_neg: bool,
    x_axis_overflow: bool,
    y_axis_overflow: bool,
}

#[derive(Debug)]
#[repr(C)]
struct MicePacket {
    header: MicePacketHeader,
    x_axis_mov: u8,
    y_axis_mov: u8,
}

impl MicePacket {
    pub const fn x_axis_change(&self) -> i16 {
        if self.header.x_axis_overflow() {
            return 0;
        }

        if self.header.x_axis_neg() {
            -(self.x_axis_mov as i16)
        } else {
            self.x_axis_mov as i16
        }
    }

    pub const fn y_axis_change(&self) -> i16 {
        if self.header.y_axis_overflow() {
            return 0;
        }

        if self.header.y_axis_neg() {
            -(self.y_axis_mov as i16)
        } else {
            self.y_axis_mov as i16
        }
    }
}

#[inline]
pub fn mice_handler() {
    static READ_DATA: SyncUnsafeCell<[u8; 3]> = SyncUnsafeCell::new([0u8; 3]);
    static READ_CURSOR: SyncUnsafeCell<usize> = SyncUnsafeCell::new(0);

    // SAFETY: only one thread will call this function at a time
    let read_data = unsafe { &mut *READ_DATA.get() };
    let read_cursor = unsafe { &mut *READ_CURSOR.get() };

    let byte0 = irq_ps2_read();
    read_data[*read_cursor] = byte0;

    *read_cursor += 1;
    if *read_cursor >= 3 {
        *read_cursor = 0;

        let received_packet: MicePacket = unsafe { core::mem::transmute(*read_data) };
        debug_assert!(
            received_packet.header.always_true(),
            "Mice packet corrupted"
        );

        let x_diff = received_packet.x_axis_change();
        let y_diff = received_packet.y_axis_change();

        let mut buttons = MiceBtnStatus::NO_BUTTONS;
        if received_packet.header.btn_left() {
            buttons = buttons.or(MiceBtnStatus::BTN_LEFT);
        }

        if received_packet.header.btn_right() {
            buttons = buttons.or(MiceBtnStatus::BTN_RIGHT);
        }

        if received_packet.header.btn_middle() {
            buttons = buttons.or(MiceBtnStatus::BTN_MID);
        }

        let event = MiceEvent {
            kind: MouseEventKind::Change,
            buttons_status: buttons,
            x_rel_change: x_diff,
            y_rel_change: y_diff,
        };

        input::mouse::on_mice_change(event);
    }
}

#[inline]
pub fn handle_ps2_keyboard() {
    use crate::drivers::keyboard;
    use keyboard::{KEYBOARD, set1::Set1Key};

    let key = irq_ps2_read();
    // outside of this function the keyboard should only be read from
    if let Some(results) = KEYBOARD
        .try_write()
        .map(|mut writer| writer.process_byte::<Set1Key>(key))
        .flatten()
    {
        match results {
            Ok(key) => keyboard::key_pressed(key),
            Err(keycode) => keyboard::key_release(keycode),
        }
    }
}

fn read_response_inner() -> u8 {
    wait_for_read();
    inb(PS2_DATA_PORT)
}
/// Sends a single byte command to the controller with no response
fn send_command(cmd: u8) {
    outb(PS2_COMMAND_PORT, cmd);
}

/// Sends a 2 byte command to the controller and receives a response
fn send_command3(cmd0: u8, cmd1: u8) -> u8 {
    outb(PS2_COMMAND_PORT, cmd0);
    wait_for_write();
    outb(PS2_DATA_PORT, cmd1);
    read_response_inner()
}

pub fn disable_controller() {
    const DISABLE_PORT1: u8 = 0xAD;
    const DISABLE_PORT2: u8 = 0xA7;

    send_command(DISABLE_PORT1);
    send_command(DISABLE_PORT2);
    // flush output buffer
    _ = inb(PS2_DATA_PORT);
}

fn wait_for_read() {
    while (inb(PS2_STATUS_PORT) & 1) == 0 {
        core::hint::spin_loop();
    }
}

fn wait_for_write() {
    while (inb(PS2_STATUS_PORT) & 2) == 2 {
        core::hint::spin_loop();
    }
}

/// From an interrupt (ex PS/2 keyboard or a PS/2 mouse) read a single byte from the controller
fn irq_ps2_read() -> u8 {
    inb(PS2_DATA_PORT)
}

pub fn setup_ps2_mouse() {
    const SET_DEFAULTS: u8 = 0xF6;
    const ENABLE_DATA_REPORTING: u8 = 0xF4;
    const ADDRESS_MOUSE: u8 = 0xD4;

    // First we write the command
    assert_eq!(
        send_command3(ADDRESS_MOUSE, SET_DEFAULTS),
        0xFA,
        "Sending command SET_DEFAULTS wasn't successful"
    );

    // Then the parameter
    assert_eq!(
        send_command3(ADDRESS_MOUSE, ENABLE_DATA_REPORTING),
        0xFA,
        "Enabling data reporting wasn't successful"
    );

    info!("PS/2 Mouse was setup successfully");
}

pub fn enable_controller() {
    const ENABLE_PORT1: u8 = 0xAE;
    const ENABLE_PORT2: u8 = 0xA8;
    // FIXME: check if the controller is a dual channel
    send_command(ENABLE_PORT1);
    send_command(ENABLE_PORT2);
}
