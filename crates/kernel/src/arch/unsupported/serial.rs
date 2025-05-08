use core::fmt::{self, Write};

use crate::utils::Locked;
use lazy_static::lazy_static;

pub struct Serial;
lazy_static! {
    /// Global Serial writer
    pub static ref SERIAL: Locked<Serial> = Locked::new(Serial);
}

impl Write for Serial {
    fn write_str(&mut self, _: &str) -> core::fmt::Result {
        todo!("serial is not implemented")
    }
}

pub fn _serial(args: fmt::Arguments) {
    SERIAL.lock().write_fmt(args).unwrap();
}
