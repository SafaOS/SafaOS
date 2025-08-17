use core::cell::SyncUnsafeCell;

use bitfield_struct::bitfield;
use safa_abi::input::{MiceBtnStatus, MiceEvent, MouseEventKind};

use crate::{
    arch::x86_64::{inb, outb},
    debug,
    devices::input,
    error, info, sleep_until, warn,
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
            (self.x_axis_mov as i16) - 0x100
        } else {
            self.x_axis_mov as i16
        }
    }

    pub const fn y_axis_change(&self) -> i16 {
        if self.header.y_axis_overflow() {
            return 0;
        }

        if self.header.y_axis_neg() {
            (self.y_axis_mov as i16) - 0x100
        } else {
            self.y_axis_mov as i16
        }
    }
}

#[inline]
pub fn mice_handler() {
    static READ_DATA: SyncUnsafeCell<[u8; 3]> = SyncUnsafeCell::new([0u8; 3]);
    static READ_CURSOR: SyncUnsafeCell<usize> = SyncUnsafeCell::new(0);
    static LAST_PACKET: SyncUnsafeCell<MicePacket> =
        SyncUnsafeCell::new(unsafe { core::mem::zeroed() });

    // SAFETY: only one thread will call this function at a time
    let read_data = unsafe { &mut *READ_DATA.get() };
    let read_cursor = unsafe { &mut *READ_CURSOR.get() };

    let byte0 = irq_ps2_read();
    if *read_cursor == 0 && (byte0 & 0b1000) == 0 {
        return;
    }

    read_data[*read_cursor] = byte0;

    *read_cursor += 1;
    if *read_cursor >= 3 {
        *read_cursor = 0;

        let last_packet = unsafe { &mut *LAST_PACKET.get() };
        let received_packet: MicePacket = unsafe { core::mem::transmute(*read_data) };

        debug_assert!(
            received_packet.header.always_true(),
            "Mice packet corrupted"
        );

        if last_packet.header.btn_left() == received_packet.header.btn_left()
            && last_packet.header.btn_middle() == received_packet.header.btn_middle()
            && last_packet.header.btn_right() == received_packet.header.btn_right()
            && received_packet.x_axis_mov == 0
            && received_packet.y_axis_mov == 0
        {
            return;
        }

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

fn read_response_inner() -> Result<u8, ()> {
    if !wait_for_read() {
        return Err(());
    }

    Ok(inb(PS2_DATA_PORT))
}
/// Sends a single byte command to the controller with no response
fn send_command(cmd: u8) {
    outb(PS2_COMMAND_PORT, cmd);
}

/// Sends a single byte command to the controller with a response
fn send_command1(cmd: u8) -> Result<u8, ()> {
    if !wait_for_write() {
        return Err(());
    }

    outb(PS2_COMMAND_PORT, cmd);
    read_response_inner()
}

/// Sends a 2 byte command to the controller without a response
fn send_command2(cmd0: u8, cmd1: u8) -> Result<(), ()> {
    outb(PS2_COMMAND_PORT, cmd0);
    if !wait_for_write() {
        return Err(());
    }
    outb(PS2_DATA_PORT, cmd1);
    Ok(())
}
/// returns Err(()) on timeout
fn send_port0_command(cmd0: u8) -> Result<(), ()> {
    if !wait_for_write() {
        return Err(());
    }
    outb(PS2_DATA_PORT, cmd0);
    Ok(())
}

/// returns Err(()) on timeout
fn send_port0_command2(cmd0: u8) -> Result<u8, ()> {
    send_port0_command(cmd0)?;
    read_response_inner()
}

/// returns Err(()) on timeout
fn send_port1_command(cmd0: u8) -> Result<(), ()> {
    send_command2(ADDRESS_MOUSE, cmd0)
}

/// returns Err(()) on timeout
fn send_port1_command2(cmd0: u8) -> Result<u8, ()> {
    send_port1_command(cmd0)?;
    read_response_inner()
}

fn disable_controller() {
    const DISABLE_PORT1: u8 = 0xAD;
    const DISABLE_PORT2: u8 = 0xA7;

    send_command(DISABLE_PORT1);
    send_command(DISABLE_PORT2);
    // flush output buffer
    while inb(PS2_STATUS_PORT) & 1 == 1 {
        inb(PS2_DATA_PORT);
    }
}

#[must_use = "Must handle timeout if it returns false"]
fn wait_for_read() -> bool {
    let success = sleep_until!(1000 ms, inb(PS2_STATUS_PORT) & 1 == 1);
    if !success {
        error!("PS/2 Controller timeout waiting for read");
    }

    success
}

#[must_use = "Must handle timeout if it returns false"]
fn wait_for_write() -> bool {
    let success = sleep_until!(1000 ms, inb(PS2_STATUS_PORT) & 2 == 0);
    if !success {
        error!("PS/2 Controller timeout waiting for write");
    }

    success
}

/// From an interrupt (ex PS/2 keyboard or a PS/2 mouse) read a single byte from the controller
fn irq_ps2_read() -> u8 {
    inb(PS2_DATA_PORT)
}

#[must_use = "Returns false if not successful"]
fn setup_ps2_keyboard() -> bool {
    const SET_SCANCODE: u8 = 0xF0;
    match send_port0_command2(SET_SCANCODE) {
        Ok(0xFA) => {}
        Ok(code) => {
            error!("Failed setting up Keyboard err: {code:#x}");
            return false;
        }
        Err(()) => return false,
    }

    match send_port0_command2(2) {
        Ok(0xFA) => {}
        Ok(code) => {
            error!("Failed setting up Keyboard err: {code:#x}");
            return false;
        }
        Err(()) => return false,
    }

    info!("Keyboard was setup successfully");
    true
}

const ADDRESS_MOUSE: u8 = 0xD4;
#[must_use = "Returns false if not successful"]
fn setup_ps2_mouse() -> bool {
    const SET_DEFAULTS: u8 = 0xF6;
    const ENABLE_DATA_REPORTING: u8 = 0xF4;

    // Set defaults
    match send_port1_command2(SET_DEFAULTS) {
        Err(()) => {
            error!("Timeout sending SET_DEFAULTS command to the PS/2 mouse");
            return false;
        }
        Ok(v) if v != 0xFA => {
            error!("SET_DEFAULTS Command on PS/2 mouse respond with err: {v:#x}");
            return false;
        }
        Ok(_) => {}
    }

    // Enable reporting data
    match send_port1_command2(ENABLE_DATA_REPORTING) {
        Ok(v) if v != 0xFA => {
            error!("ENABLE_DATA_REPORTING Command on PS/2 mouse responded with err: {v:#x}");
            return false;
        }
        Err(()) => {
            error!("Timeout sending ENABLE_DATA_REPORTING command to PS/2 mouse");
            return false;
        }
        Ok(_) => {}
    }

    info!("PS/2 Mouse was setup successfully");
    true
}

#[bitfield(u8)]
struct ConfByte {
    port0_interrupt_enabled: bool,
    port1_interrupt_enabled: bool,

    should_be_one: bool,
    should_be_zero: bool,
    port0_clock_disabled: bool,
    port1_clock_disabled: bool,
    port0_translation: bool,
    must_be_zero: bool,
}

fn read_conf_byte() -> Result<ConfByte, ()> {
    const READ_CONF_BYTE: u8 = 0x20;
    send_command1(READ_CONF_BYTE).map(|ok| ConfByte::from_bits(ok))
}

fn write_conf_byte(byte: ConfByte) -> Result<(), ()> {
    const WRITE_CONF_BYTE: u8 = 0x60;
    send_command2(WRITE_CONF_BYTE, byte.into_bits())
}

const ENABLE_PORT0: u8 = 0xAE;
const ENABLE_PORT1: u8 = 0xA8;

fn self_test() -> Result<(), ()> {
    const CONTROLLER_SELF_TEST: u8 = 0xAA;

    let response = send_command1(CONTROLLER_SELF_TEST)?;
    if response != 0x55 {
        error!("PS/2 Controller self test failed with err: {response:#x}");
        return Err(());
    }

    Ok(())
}

fn is_dual_channel() -> Result<bool, ()> {
    send_command(ENABLE_PORT1);

    let conf = read_conf_byte()?;
    let results = !conf.port1_clock_disabled();
    write_conf_byte(
        conf.with_port1_interrupt_enabled(false)
            .with_port0_interrupt_enabled(false)
            .with_port0_translation(false),
    )?;
    Ok(results)
}

fn test_port0() -> Result<bool, ()> {
    const TEST_PORT0: u8 = 0xAB;
    let results = send_command1(TEST_PORT0)?;
    let success = results == 0;

    if !success {
        warn!("Port0 failed tests with err: {results:?}, skipping...");
        return Err(());
    }
    Ok(success)
}

fn test_port1() -> Result<bool, ()> {
    const TEST_PORT1: u8 = 0xA9;
    let results = send_command1(TEST_PORT1)?;
    let success = results == 0;

    if !success {
        warn!("Port1 failed tests with err: {results:?}, skipping...");
        return Err(());
    }
    Ok(success)
}

/// If this returns an error the port must be treated as non-existent
fn reset_devices(port0: bool) -> Result<(), ()> {
    let byte0 = if port0 {
        send_port0_command2(0xFF)?
    } else {
        send_port1_command2(0xFF)?
    };

    if byte0 == 0xFC {
        return Err(());
    }

    let byte1 = read_response_inner()?;

    if byte0 == 0xFA && byte1 == 0xFC
        || !((byte0 == 0xFA && byte1 == 0xAA) || (byte0 == 0xAA && byte1 == 0xFA))
    {
        return Err(());
    }

    let id = read_response_inner().ok();
    info!(
        "PS/2 Port {} reset, device with id: {id:#x?}",
        if port0 { 1 } else { 2 }
    );
    Ok(())
}

/// Setups PS/2 Controller, returns an Err(()) if setup failed or
/// Ok((can_use_keyboard, can_use_mice)) if setup was successful
pub fn setup_controller() -> Result<(bool, bool), ()> {
    info!("Setting up PS/2 Controller");

    disable_controller();

    let conf_byte = read_conf_byte()?
        .with_port0_interrupt_enabled(false)
        .with_port1_interrupt_enabled(false)
        .with_port0_translation(false);

    write_conf_byte(conf_byte)?;
    self_test()?;

    let is_dual_channel = match is_dual_channel() {
        Ok(b) => b,
        Err(()) => {
            error!("Timeout checking if the PS/2 controller is a dual channel");
            return Err(());
        }
    };

    debug!("PS/2 Controller is dual channel: {is_dual_channel}");

    let can_use_port0 = match test_port0() {
        Ok(k) => k,
        Err(()) => {
            error!("Timeout testing PS/2 Controller's Port0");
            return Err(());
        }
    };

    /* short circuits */
    let can_use_port1 = is_dual_channel
        && match test_port1() {
            Ok(k) => k,
            Err(()) => {
                error!("Timeout testing PS/2 Controller's Port1");
                return Err(());
            }
        };

    if can_use_port0 {
        send_command(ENABLE_PORT0);
        debug!("Enabled Port0");
    }

    if can_use_port1 {
        send_command(ENABLE_PORT1);
        debug!("Eanbled Port1");
    }

    let can_use_device0 = can_use_port0 && reset_devices(true).is_ok();
    if !can_use_device0 {
        warn!("PS/2 Cannot use device 0");
    }

    let can_use_device1 = can_use_port1 && reset_devices(false).is_ok();
    if !can_use_device1 {
        warn!("PS/2 Cannot use device 1")
    }

    let can_use_keyboard = can_use_device0 && {
        let success = setup_ps2_keyboard();
        if !success {
            error!("Setting up keyboard in Port1 failed");
        }

        success
    };

    let can_use_mouse = can_use_device1 && {
        let success = setup_ps2_mouse();
        if !success {
            error!("Setting up mouse in Port1 failed");
        }

        success
    };

    let conf_byte = read_conf_byte()?;
    let conf_byte = conf_byte
        .with_port0_interrupt_enabled(true)
        .with_port1_interrupt_enabled(true)
        .with_port0_translation(true);
    write_conf_byte(conf_byte)?;

    Ok((can_use_keyboard, can_use_mouse))
}
