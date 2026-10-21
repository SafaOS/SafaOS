//! Small framebuffer abstraction layer.
use core::ptr::NonNull;

#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub base: NonNull<()>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Bytes per row (width may not be properly aligned).
    pub pitch: usize,
    /// FIXME: Only supports XBGR and XRGB
    pub format: PixelFormat,
}
unsafe impl Sync for FramebufferInfo {}
unsafe impl Send for FramebufferInfo {}

#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    /// [0] => blue, [1] => green. [2] => red
    BgrX888,
    /// [0] => red, [1] => green. [2] => blue
    RgbX888,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(&self) -> u8 {
        match self {
            Self::BgrX888 => 4,
            Self::RgbX888 => 4,
        }
    }
}

/// Returns information about the best framebuffer for direct CPU drawing
#[inline]
pub fn framebuffer() -> Option<FramebufferInfo> {
    super::current::framebuffer()
}
