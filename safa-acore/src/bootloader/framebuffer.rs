//! Small framebuffer abstraction layer.
use core::ptr::NonNull;

use crate::oninit;

#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub base: NonNull<()>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Bits per pixel
    pub bpp: u16,
    /// Bytes per row (width may not be properly aligned).
    pub pitch: usize,
    /// FIXME: Only supports XBGR and XRGB
    pub format: PixelFormat,
}
unsafe impl Sync for FramebufferInfo {}

#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    /// [0] => blue, [1] => green. [2] => red
    Bgr888,
    /// [0] => red, [1] => green. [2] => blue
    Rgb888,
}

oninit::define! {
    pub static FRAMEBUFFER_INFO: Option<FramebufferInfo> = || framebuffer();
}

/// Returns information about the best framebuffer for direct CPU drawing
#[inline]
pub fn framebuffer() -> Option<FramebufferInfo> {
    super::current::framebuffer()
}
