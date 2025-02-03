use core::{fmt::Write, str};

use spin::RwLock;

use crate::{
    drivers::vfs::{FSError, FSResult},
    terminal::{TTYInterface, TTY},
};

use super::CharDevice;

impl<T: TTYInterface> CharDevice for RwLock<TTY<T>> {
    fn name(&self) -> &'static str {
        "tty"
    }

    fn read(&self, buffer: &mut [u8]) -> FSResult<usize> {
        let lock = self.try_write();

        if let Some(mut tty) = lock {
            tty.enable_input();

            if tty.stdin_buffer.ends_with('\n') {
                tty.disable_input();
                let count = tty.stdin_buffer.len().min(buffer.len());
                buffer[..count].copy_from_slice(&tty.stdin_buffer.as_bytes()[..count]);
                tty.stdin_buffer.drain(..count);

                return Ok(count);
            }
        }

        Err(FSError::ResourceBusy)
    }

    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        unsafe {
            let _ = self
                .try_write()
                .ok_or(FSError::ResourceBusy)?
                .write_str(&str::from_utf8_unchecked(buffer));
        }
        Ok(buffer.len())
    }

    fn sync(&self) -> FSResult<()> {
        let mut writer = self.try_write().ok_or(FSError::ResourceBusy)?;
        writer.sync();
        Ok(())
    }
}
