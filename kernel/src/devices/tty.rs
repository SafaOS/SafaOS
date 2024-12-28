use core::fmt::Write;

use alloc::string::String;
use spin::RwLock;

use crate::{
    drivers::vfs::{FSError, FSResult},
    terminal::TTY,
    threading::expose::thread_yeild,
};

use super::CharDevice;

impl CharDevice for RwLock<TTY<'_>> {
    fn name(&self) -> &'static str {
        "tty"
    }

    fn read(&self, buffer: &mut [u8]) -> FSResult<usize> {
        if !self
            .try_read()
            .ok_or(FSError::ResourceBusy)?
            .stdin_buffer
            .ends_with(['\n'])
        {
            self.write().enable_input();
            while self
                .try_read()
                .is_none_or(|reader| !reader.stdin_buffer.ends_with(['\n']))
            {
                thread_yeild();
            }

            self.write().disable_input();
        }

        let stdin_buffer = &mut self.write().stdin_buffer;

        let count = if stdin_buffer.len() <= buffer.len() {
            stdin_buffer.len()
        } else {
            buffer.len()
        };

        buffer[..count].copy_from_slice(&stdin_buffer.as_str().as_bytes()[..count]);
        stdin_buffer.inner.drain(..count);
        Ok(count)
    }

    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        let _ = self
            .try_write()
            .ok_or(FSError::ResourceBusy)?
            .write_str(&String::from_utf8_lossy(buffer));
        Ok(buffer.len())
    }
}
