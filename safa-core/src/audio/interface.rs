use alloc::boxed::Box;

use crate::{
    devices::CharDevice,
    drivers::vfs::{FSError, FSResult},
    syscalls::ffi::SyscallFFI,
};

/// Describes the PCM Format an Audio card accepts.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioInfo {
    freq_hz: u32,
    __rsvd: [u8; 1],
    stride_per_sample: u8,
    bits_per_sample: u8,
    channels: u8,
}

impl AudioInfo {
    pub const fn bytes_per_sample(&self) -> usize {
        self.stride_per_sample as usize / 8
    }

    pub const fn bytes_per_frame(&self) -> usize {
        self.bytes_per_sample() * self.channels as usize
    }
    pub const fn new(freq: u32, sample_bits: u8, stride_per_sample: u8, channels: u8) -> Self {
        Self {
            freq_hz: freq,
            bits_per_sample: sample_bits,
            __rsvd: [0u8; 1],
            stride_per_sample,
            channels,
        }
    }
}

/// Describes an AudioCard device interface.
pub trait AudioCard: Send + Sync {
    fn name(&self) -> &'static str;
    fn info(&self) -> AudioInfo;
    fn transfer_buf_size(&self) -> usize;
    fn queued_samples_count(&self) -> usize;
    fn transfer_data(&self, data: &[u8]) -> FSResult<usize>;
}

const CMD_GET_AC_AUDIO_INFO: u16 = 0x1001;
const CMD_GET_AC_BUF_SIZE: u16 = 0x1002;
const CMD_GET_AC_QUEUED_SAMPLES: u16 = 0x1003;

pub struct BoxAudioDev(pub Box<dyn AudioCard>);

impl CharDevice for BoxAudioDev {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn write(&self, buffer: &[u8]) -> crate::drivers::vfs::FSResult<usize> {
        self.0.transfer_data(buffer)
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

            CMD_GET_AC_QUEUED_SAMPLES => {
                let ptr: &mut usize =
                    SyscallFFI::make(arg as *mut usize).map_err(|_| FSError::InvalidArg)?;
                *ptr = self.0.queued_samples_count();
                Ok(())
            }
            _ => Err(FSError::InvalidCmd),
        }
    }
}

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

            CMD_GET_AC_QUEUED_SAMPLES => {
                let ptr: &mut usize =
                    SyscallFFI::make(arg as *mut usize).map_err(|_| FSError::InvalidArg)?;
                *ptr = self.0.queued_samples_count();
                Ok(())
            }
            _ => Err(FSError::InvalidCmd),
        }
    }
}
