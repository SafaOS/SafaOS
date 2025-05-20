use core::fmt::{self, Write};

use crate::limine::HHDM;
use crate::utils::Locked;
use crate::PhysAddr;
use lazy_static::lazy_static;

pub const UART_ADDR: PhysAddr = 0x09000000;
// TODO: device trees and figure this out from there?
lazy_static! {
    static ref UART: usize = *HHDM + UART_ADDR;
}

#[inline(always)]
fn putbyte(c: u8) {
    unsafe {
        core::ptr::write_volatile(*UART as *mut u8, c);
    }
}

fn putc(c: char) {
    // FIXME: utf8?
    putbyte(c as u8);
}

pub(super) fn write_str(s: &str) {
    for c in s.chars() {
        putc(c);
    }
}

pub struct Serial;
lazy_static! {
    /// Global Serial writer
    pub static ref SERIAL: Locked<Serial> = Locked::new(Serial);
}

impl Write for Serial {
    fn write_char(&mut self, c: char) -> fmt::Result {
        putc(c);
        Ok(())
    }

    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

pub fn _serial(args: fmt::Arguments) {
    SERIAL.lock().write_fmt(args).unwrap();
}
