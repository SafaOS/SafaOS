use crate::{
    devices::{BlockDevice, Device},
    drivers::{
        framebuffer::FrameBufferDriver,
        vfs::{FSError, SeekOffset},
    },
    process::vas::MemMappedInterface,
    syscalls::ffi::SyscallFFI,
};
use alloc::boxed::Box;
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
    SyncRect,
}

impl MemMappedInterface for FrameBufferDriver {
    fn frames(&self) -> &[crate::memory::frame_allocator::Frame] {
        self.frames()
    }

    fn sync(&self) -> crate::drivers::vfs::FSResult<()> {
        self.buffer().sync_pixels_full();
        Ok(())
    }

    fn send_command(&self, cmd: u16, arg: u64) -> crate::drivers::vfs::FSResult<()> {
        Device::send_command(self, cmd, arg)
    }
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
                self.buffer().sync_pixels_full();
            }
            Cmd::SyncRect => {
                #[derive(Debug, Clone, Copy)]
                #[repr(C)]
                struct SyncRect {
                    off_x: usize,
                    off_y: usize,
                    width: usize,
                    height: usize,
                }

                let ptr = arg as *const SyncRect;
                let rect = unsafe { *ptr };
                self.buffer()
                    .sync_pixels_rect(rect.off_x, rect.off_y, rect.width, rect.height);
            }
        }

        Ok(())
    }

    fn mmap(
        &self,
        offset: SeekOffset,
        page_count: usize,
    ) -> crate::drivers::vfs::FSResult<alloc::boxed::Box<dyn crate::process::vas::MemMappedInterface>>
    {
        // FIXME: offset and page counts are ignored for now
        _ = offset;
        _ = page_count;
        let new = FrameBufferDriver::create(1);
        Ok(Box::new(new))
    }
}

impl BlockDevice for FrameBufferDriver {}
