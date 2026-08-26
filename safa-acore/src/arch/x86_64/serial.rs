use super::io::{inb, outb};

pub const SERIAL_COM1_BASE: u16 = 0x3F8;

const SERIAL_DATA_PORT: u16 = SERIAL_COM1_BASE;
const SERIAL_FIFO_COMMAND_PORT: u16 = SERIAL_COM1_BASE + 2;
const SERIAL_LINE_COMMAND_PORT: u16 = SERIAL_COM1_BASE + 3;
const SERIAL_MODEM_COMMAND_PORT: u16 = SERIAL_COM1_BASE + 4;
const SERIAL_LINE_STATUS_PORT: u16 = SERIAL_COM1_BASE + 5;

const SERIAL_LINE_ENABLE_DLAB: u8 = 0x80;

pub type SerialInner = ();
pub const fn new_serial() -> SerialInner {
    ()
}

pub(super) fn init_serial_inner() {
    unsafe {
        outb(SERIAL_DATA_PORT + 1, 0x00);
        outb(SERIAL_LINE_COMMAND_PORT, SERIAL_LINE_ENABLE_DLAB);
        outb(SERIAL_DATA_PORT, 0x03);
        outb(SERIAL_DATA_PORT + 1, 0x00);

        outb(SERIAL_LINE_COMMAND_PORT, 0x3);
        outb(SERIAL_FIFO_COMMAND_PORT, 0xC7);
        outb(SERIAL_MODEM_COMMAND_PORT, 0x0B);
        outb(SERIAL_MODEM_COMMAND_PORT, 0x1E);

        outb(SERIAL_DATA_PORT, 0xAE);

        outb(SERIAL_MODEM_COMMAND_PORT, 0x0F);
    }
    write_serial_string(&mut (), "\nSafaOS: Serial initialized\n");
}

pub fn init_serial(inner: &mut SerialInner) -> Result<(), &'static str> {
    _ = inner;
    Ok(())
}

pub fn serial_is_transmit_fifo_empty() -> bool {
    unsafe { (inb(SERIAL_LINE_STATUS_PORT) & 0x20) != 0 }
}

pub fn write_serial(byte: u8) {
    // Wait for the FIFO buffer to be empty
    while !serial_is_transmit_fifo_empty() {}
    unsafe {
        if byte == b'\n' {
            outb(SERIAL_DATA_PORT, b'\r');
        }
        outb(SERIAL_DATA_PORT, byte);
    }
}

pub fn write_serial_string(_inner: &mut SerialInner, s: &str) {
    for byte in s.bytes() {
        write_serial(byte);
    }
}
