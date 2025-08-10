use crate::{
    devices::{BlockDevice, Device},
    drivers::{
        framebuffer::FrameBufferDriver,
        vfs::{FSError, SeekOffset},
    },
    syscalls::ffi::SyscallFFI,
};
use int_enum::IntEnum;

#[repr(C)]
/// A struct represinting information about the virtual framebuffer
pub struct FramebufferDevInfo {
    width: usize,
    height: usize,
    /// Bits per pixel, for now the virtual framebuffer always have 32bits per pixel
    bpp: usize,
    /// Whether or not each pixel is encoded as BGR and not RGB (always false for now)
    bgr: bool,
}

#[derive(Debug, Clone, Copy, IntEnum)]
#[repr(u16)]
enum Cmd {
    Sync,
    GetInfo,
}

impl Device for FrameBufferDriver {
    fn name(&self) -> &'static str {
        "fb"
    }

    fn read(&self, offset: SeekOffset, buffer: &mut [u8]) -> crate::drivers::vfs::FSResult<usize> {
        // TODO: Implement Read, for now we only support mapping this device to memory
        _ = offset;
        _ = buffer;
        Err(FSError::OperationNotSupported)
    }
    fn write(&self, offset: SeekOffset, buffer: &[u8]) -> crate::drivers::vfs::FSResult<usize> {
        self.buffer().write_bytes(offset, buffer)
    }

    /// Performs a full pixel Sync
    fn sync(&self) -> crate::drivers::vfs::FSResult<()> {
        self.buffer().sync_pixels_full();
        Ok(())
    }

    fn send_command(&self, cmd: u16, arg: u64) -> crate::drivers::vfs::FSResult<()> {
        let cmd = Cmd::try_from(cmd).map_err(|_| FSError::InvalidCmd)?;
        match cmd {
            Cmd::GetInfo => {
                core::hint::cold_path();

                let ptr = <&mut FramebufferDevInfo>::make(arg as *mut FramebufferDevInfo)
                    .map_err(|_| FSError::InvalidArg)?;

                *ptr = FramebufferDevInfo {
                    height: self.height(),
                    width: self.width(),
                    bpp: 32,
                    bgr: false,
                };
            }
            Cmd::Sync => {
                // the lower 32 bits of the argument is the start pixel
                // the higher 32 bits is the amount of pixels to sync
                let start_pixel = (arg & (u32::MAX as u64)) as usize;
                let count = ((arg >> 32) & (u32::MAX as u64)) as usize;
                self.buffer().sync_pixels(start_pixel, count);
            }
        }

        Ok(())
    }
}

impl BlockDevice for FrameBufferDriver {}
