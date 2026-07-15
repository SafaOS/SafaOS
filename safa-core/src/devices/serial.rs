use crate::{
    drivers::vfs::{FSError, FSResult},
    serial_log,
};

use super::CharDevice;
pub struct SerialDevice;

impl CharDevice for SerialDevice {
    fn name(&self) -> &'static str {
        "ss"
    }

    fn read(&self, _buffer: &mut [u8]) -> FSResult<usize> {
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        let str = unsafe { core::str::from_utf8_unchecked(buffer) };
        serial_log!("{}", str.trim_end_matches('\n'));
        Ok(buffer.len())
    }
}
