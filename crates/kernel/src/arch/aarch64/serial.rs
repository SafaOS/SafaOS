use core::fmt::{self, Write};
use core::hint::unlikely;

use crate::utils::locks::SpinLock;
use crate::{PhysAddr, VirtAddr};
use lazy_static::lazy_static;
lazy_static! {
    static ref PL011: VirtAddr = super::cpu::PL011BASE.into_virt();
}

#[inline(always)]
fn putbyte(c: u8) {
    unsafe {
        if unlikely(!super::cpu::serial_ready()) {
            // hack to allow debug prints before the DTB is parsed
            let qemu_addr = PhysAddr::from(0x09000000).into_virt();
            core::ptr::write_volatile(qemu_addr.into_ptr::<u8>(), c);
        } else {
            core::ptr::write_volatile(PL011.into_ptr(), c);
        }
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
    pub static ref SERIAL: SpinLock<Serial> = SpinLock::new(Serial);
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
    super::without_interrupts(|| SERIAL.lock().write_fmt(args).unwrap())
}
