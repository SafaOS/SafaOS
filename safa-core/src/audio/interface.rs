use crate::{devices::CharDevice, drivers::vfs::FSError, syscalls::ffi::SyscallFFI};

/// Describes the PCM Format an Audio card accepts.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioInfo {
    freq_hz: u32,
    __padding: [u8; 2],
    bits_per_sample: u8,
    channels: u8,
}

impl AudioInfo {
    pub const fn new(freq: u32, sample_bits: u8, channels: u8) -> Self {
        Self {
            freq_hz: freq,
            bits_per_sample: sample_bits,
            __padding: [0u8; 2],
            channels,
        }
    }
}

/// Describes an AudioCard device interface.
pub trait AudioCard: Send + Sync {
    fn name(&self) -> &'static str;
    fn info(&self) -> AudioInfo;
    fn transfer_buf_size(&self) -> usize;
    fn transfer_data(&self, data: &[u8]) -> Result<usize, ()>;
}

const CMD_GET_AC_AUDIO_INFO: u16 = 0x1001;
const CMD_GET_AC_BUF_SIZE: u16 = 0x1002;

/// An audio device that represents a reference to an AudioCard.
pub struct AudioDev<'a>(pub &'a dyn AudioCard);

impl<'a> CharDevice for AudioDev<'a> {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn write(&self, buffer: &[u8]) -> crate::drivers::vfs::FSResult<usize> {
        Ok(self.0.transfer_data(buffer).unwrap_or(0))
    }
    fn read(&self, buffer: &mut [u8]) -> crate::drivers::vfs::FSResult<usize> {
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }
    fn send_command(&self, cmd: u16, arg: u64) -> crate::drivers::vfs::FSResult<()> {
        match cmd {
            CMD_GET_AC_AUDIO_INFO => {
                let ptr: &mut AudioInfo =
                    SyscallFFI::make(arg as *mut AudioInfo).map_err(|_| FSError::InvalidArg)?;
                *ptr = self.0.info();
                Ok(())
            }
            CMD_GET_AC_BUF_SIZE => {
                let ptr: &mut usize =
                    SyscallFFI::make(arg as *mut usize).map_err(|_| FSError::InvalidArg)?;
                *ptr = self.0.transfer_buf_size();
                Ok(())
            }
            _ => Err(FSError::InvalidCmd),
        }
    }
}
