use core::{fmt::Write, str};

use spin::RwLock;

use crate::{
    drivers::vfs::FSResult,
    terminal::{TTYInterface, TTY},
    threading::expose::thread_yeild,
};

use super::CharDevice;

impl<T: TTYInterface> CharDevice for RwLock<TTY<T>> {
    fn name(&self) -> &'static str {
        "tty"
    }

    fn read(&self, buffer: &mut [u8]) -> FSResult<usize> {
        loop {
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

            thread_yeild();
        }
    }

    fn write(&self, buffer: &[u8]) -> FSResult<usize> {
        let str = unsafe { &str::from_utf8_unchecked(buffer) };
        loop {
            let lock = self.try_write();

            match lock {
                Some(mut tty) => {
                    tty.write_str(str).unwrap();
                    return Ok(buffer.len());
                }
                None => thread_yeild(),
            }
        }
    }

    fn sync(&self) -> FSResult<()> {
        loop {
            match self.try_write() {
                Some(mut writer) => {
                    writer.sync();
                    return Ok(());
                }
                None => thread_yeild(),
            }
        }
    }
}
