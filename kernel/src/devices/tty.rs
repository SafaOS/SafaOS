use core::{fmt::Write, str};

use int_enum::IntEnum;
use spin::RwLock;

use crate::{
    drivers::vfs::{CtlArgs, FSError, FSResult},
    terminal::{TTYInterface, TTYSettings, TTY},
    threading::expose::thread_yeild,
};

use super::CharDevice;

#[derive(Debug, IntEnum)]
#[repr(u16)]
pub enum TTYCtlCmd {
    GetFlags = 0,
    SetFlags = 1,
}

impl<T: TTYInterface> CharDevice for RwLock<TTY<T>> {
    fn name(&self) -> &'static str {
        "tty"
    }

    fn read(&self, buffer: &mut [u8]) -> FSResult<usize> {
        loop {
            let lock = self.try_write();

            if let Some(mut tty) = lock {
                if (tty.stdin_buffer.ends_with('\n')
                    && tty.settings.contains(TTYSettings::CANONICAL_MODE))
                    || (!tty.stdin_buffer.is_empty()
                        && !tty.settings.contains(TTYSettings::CANONICAL_MODE))
                {
                    tty.disable_input();
                    let count = tty.stdin_buffer.len().min(buffer.len());
                    buffer[..count].copy_from_slice(&tty.stdin_buffer.as_bytes()[..count]);
                    tty.stdin_buffer.drain(..count);

                    return Ok(count);
                }

                tty.enable_input();
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

    fn ctl(&self, cmd: u16, mut args: CtlArgs) -> FSResult<()> {
        let cmd = TTYCtlCmd::try_from(cmd).map_err(|_| FSError::InvaildCtlCmd)?;
        match cmd {
            TTYCtlCmd::GetFlags => {
                let flags = args.get_ref_to::<TTYSettings>()?;
                *flags = self.read().settings;
                Ok(())
            }
            TTYCtlCmd::SetFlags => {
                let flags: TTYSettings =
                    TTYSettings::from_bits(args.get_ty()?).ok_or(FSError::InvaildCtlArg)?;
                self.write().settings = flags;
                Ok(())
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
