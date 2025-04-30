use crate::utils::locks::{Mutex, MutexGuard};
use alloc::{boxed::Box, vec::Vec};
use lazy_static::lazy_static;

use crate::{
    debug, limine,
    memory::page_allocator::{PageAlloc, GLOBAL_PAGE_ALLOCATOR},
    utils::display::RGB,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    #[allow(dead_code)]
    /// TODO: use
    Bgr,
}
#[derive(Debug, Clone, Copy)]
pub struct FrameBufferInfo {
    /// number of pixels between start of a line and another
    pub stride: usize,
    pub height: usize,
    pub bytes_per_pixel: usize,
    pub _pixel_format: PixelFormat,
}

pub struct FrameBuffer<'a> {
    info: FrameBufferInfo,
    buffer_display_index: usize,
    buffer: Box<[u32], PageAlloc>,
    video_buffer: &'a mut [u32],
}

impl<'a> FrameBuffer<'a> {
    pub fn new(video_buffer: &'a mut [u32], info: FrameBufferInfo) -> Self {
        let mut buffer = Vec::with_capacity_in(video_buffer.len() * 4, &*GLOBAL_PAGE_ALLOCATOR);
        unsafe {
            buffer.set_len(buffer.capacity());
        }

        let buffer = buffer.into_boxed_slice();
        debug!(FrameBuffer, "created ({}KiB)", buffer.len() / 1024);

        Self {
            info,
            buffer_display_index: 0,
            buffer,
            video_buffer,
        }
    }

    #[inline(always)]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: RGB) {
        let index = x + y * self.info.stride;
        self.buffer[self.buffer_display_index + index] = color.into_u32();
    }

    /// draws all pixels in the buffer to the actual video_buffer
    pub fn sync_pixels(&mut self) {
        self.video_buffer.copy_from_slice(
            &self.buffer
                [self.buffer_display_index..self.buffer_display_index + self.video_buffer.len()],
        );
    }

    #[inline]
    /// shifts the buffer by `pixels` pixels
    /// can be used to achieve scrolling
    /// ensures that there are self.width() * self.height() pixels to draw
    pub fn shift_buffer(&mut self, pixels: isize) {
        match pixels.cmp(&0) {
            core::cmp::Ordering::Less => {
                let amount = -pixels as usize;
                self.buffer_display_index = self.buffer_display_index.saturating_sub(amount);
            }
            core::cmp::Ordering::Greater => {
                let amount = pixels as usize;
                let max_index = self.buffer.len() - self.video_buffer.len();
                let new_index = self.buffer_display_index + amount;

                if new_index <= max_index {
                    self.buffer_display_index = new_index;
                } else {
                    self.buffer_display_index = max_index;
                    self.buffer.copy_within(amount.., 0);
                }
            }
            core::cmp::Ordering::Equal => {}
        }

        self.sync_pixels();
    }

    #[inline(always)]
    /// sets the cursor to `pixel` in pixels
    pub fn set_cursor(&mut self, pixel: usize) {
        self.buffer_display_index = pixel;
    }

    /// FIXME: assumes that [`self.info.bytes_per_pixel`] == 4
    pub fn fill(&mut self, color: RGB) {
        let color: u32 = color.into();
        self.buffer.fill(color);
    }
}
pub struct FrameBufferDriver {
    info: FrameBufferInfo,
    inner: Mutex<FrameBuffer<'static>>,
}

impl FrameBufferDriver {
    pub fn create() -> Self {
        let (video_buffer, info) = limine::get_framebuffer();
        assert_eq!(info.bytes_per_pixel, 4);
        let framebuffer = FrameBuffer::new(video_buffer, info);

        Self {
            info,
            inner: Mutex::new(framebuffer),
        }
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.info.stride
    }

    #[inline(always)]
    pub fn height(&self) -> usize {
        self.info.height
    }

    #[inline]
    pub fn buffer(&self) -> MutexGuard<FrameBuffer<'static>> {
        self.inner.lock()
    }
}

lazy_static! {
    pub static ref FRAMEBUFFER_DRIVER: FrameBufferDriver = FrameBufferDriver::create();
}
