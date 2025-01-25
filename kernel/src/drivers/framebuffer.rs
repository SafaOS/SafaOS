use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::RwLock;

use crate::{
    limine,
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
#[derive(Debug)]
pub struct FrameBufferInfo {
    /// number of pixels between start of a line and another
    pub stride: usize,
    pub bytes_per_pixel: usize,
    pub pixel_format: PixelFormat,
}

pub struct FrameBuffer {
    pub info: FrameBufferInfo,
    buffer_display_index: usize,
    buffer: Vec<u8, PageAlloc>,
    video_buffer: &'static mut [u8],
}

impl FrameBuffer {
    pub fn new() -> Self {
        let (video_buffer, info) = limine::get_framebuffer();
        let mut buffer = Vec::with_capacity_in(video_buffer.len() * 4, &*GLOBAL_PAGE_ALLOCATOR);
        buffer.resize(video_buffer.len() * 4, 0);
        Self {
            info,
            buffer_display_index: 0,
            buffer,
            video_buffer,
        }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: RGB) {
        let index = x + y * self.info.stride;
        let mut bytes = color.bytes();

        if self.info.pixel_format == PixelFormat::Rgb {
            bytes.reverse();
        }
        self.buffer
            [index * self.info.bytes_per_pixel..index * self.info.bytes_per_pixel + bytes.len()]
            .copy_from_slice(&bytes);
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
    /// can be used to achive scrolling
    pub fn shift_buffer(&mut self, pixels: isize) {
        match pixels.cmp(&0) {
            core::cmp::Ordering::Less => {
                let amount = (-pixels as usize) * self.info.bytes_per_pixel;
                if amount > self.buffer_display_index {
                    self.buffer_display_index = 0;
                    return;
                }

                self.buffer_display_index -= amount;
            }
            core::cmp::Ordering::Greater => {
                let amount = pixels as usize * self.info.bytes_per_pixel;
                if amount + self.buffer_display_index >= self.buffer.len() - self.video_buffer.len()
                {
                    self.buffer_display_index = self.buffer.len() - self.video_buffer.len();
                    return;
                }

                self.buffer_display_index += amount;
            }
            core::cmp::Ordering::Equal => {}
        }

        self.sync_pixels();
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.info.stride
    }
    #[inline(always)]
    pub fn height(&self) -> usize {
        self.video_buffer.len() / self.info.bytes_per_pixel / self.width()
    }

    #[inline(always)]
    /// returns the current draw cursor position in pixels
    pub fn get_cursor(&self) -> usize {
        self.buffer_display_index / self.info.bytes_per_pixel
    }

    #[inline(always)]
    /// sets the cursor to `pixel` in pixels
    pub fn set_cursor(&mut self, pixel: usize) {
        self.buffer_display_index = pixel * self.info.bytes_per_pixel;
    }

    #[inline(always)]
    /// clears the framebuffer
    pub fn clear(&mut self) {
        self.buffer.fill(0);
    }
}

lazy_static! {
    pub static ref FRAMEBUFFER_DRIVER: RwLock<FrameBuffer> = RwLock::new(FrameBuffer::new());
}
