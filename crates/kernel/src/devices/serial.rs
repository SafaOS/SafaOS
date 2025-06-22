use core::fmt::Write;

use crate::{
    arch::serial::Serial,
    drivers::vfs::{FSError, FSResult},
    threading::expose::thread_yield,
    utils::locks::Mutex,
};

use super::CharDevice;

impl CharDevice for Mutex<Serial> {
    fn name(&self) -> &'static str {
        "ss"
    }

    fn read(&self, _buffer: &mut [u8]) -> FSResult<usize> {
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        let str = unsafe { core::str::from_utf8_unchecked(buffer) };
        loop {
            match self.try_lock() {
                Some(mut writer) => {
                    writer.write_str(str).unwrap();
                    return Ok(buffer.len());
                }
                None => thread_yield(),
            }
        }
    }
}
