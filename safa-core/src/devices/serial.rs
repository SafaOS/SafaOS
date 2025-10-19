use core::fmt::Write;

use crate::{
    arch::serial::Serial,
    drivers::vfs::{FSError, FSResult},
    utils::locks::SpinLock,
};

use super::CharDevice;

impl CharDevice for SpinLock<Serial> {
    fn name(&self) -> &'static str {
        "ss"
    }

    fn read(&self, _buffer: &mut [u8]) -> FSResult<usize> {
        FSResult::Err(FSError::OperationNotSupported)
    }

    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        let str = unsafe { core::str::from_utf8_unchecked(buffer) };
        self.lock()
            .write_str(str)
            .expect("failed to write to serial");
        Ok(buffer.len())
    }
}
