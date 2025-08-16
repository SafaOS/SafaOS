use crate::{
    VirtAddr,
    arch::paging::current_higher_root_table,
    drivers::vfs::{FSError, FSResult, SeekOffset},
    memory::{
        frame_allocator::Frame,
        paging::{PAGE_SIZE, Page},
    },
    utils::locks::{Mutex, MutexGuard},
};
use alloc::{boxed::Box, vec::Vec};
use lazy_static::lazy_static;

use crate::{debug, limine, memory::page_allocator::PageAlloc, utils::display::RGB};

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
    pixel_buffer: Box<[u32], PageAlloc>,
    video_buffer: &'a mut [u8],
}

impl<'a> FrameBuffer<'a> {
    pub fn new(
        video_buffer: &'a mut [u8],
        pixels_buffers_count: usize,
        info: FrameBufferInfo,
    ) -> Self {
        let mut pixel_buffer = Vec::with_capacity_in(
            (info.stride * info.height) * pixels_buffers_count,
            PageAlloc,
        );
        unsafe {
            pixel_buffer.set_len(pixel_buffer.capacity());
        }

        let pixel_buffer = pixel_buffer.into_boxed_slice();
        debug!(
            FrameBuffer,
            "created ({}KiB)",
            pixel_buffer.len() * 4 / 1024
        );

        Self {
            info,
            buffer_display_index: 0,
            pixel_buffer,
            video_buffer,
        }
    }

    #[inline(always)]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: RGB) {
        let index = x + y * self.info.stride;
        self.pixel_buffer[self.buffer_display_index + index] = color.into_u32();
    }

    /// Writes the given bytes buffer to the framebuffer
    pub fn write_bytes(&mut self, offset: SeekOffset, bytes: &[u8]) -> FSResult<usize> {
        let pixels = &self.pixel_buffer[self.buffer_display_index..];
        let pb_ptr = pixels.as_ptr();
        let pb_len = pixels.len();

        let pb_u8_ptr = pb_ptr as *mut u8;
        let pb_u8_len = pb_len * size_of::<u32>();
        let pb_u8_buf = unsafe { core::slice::from_raw_parts_mut(pb_u8_ptr, pb_u8_len) };
        let pb_u8_buf = match offset {
            SeekOffset::Start(0) => pb_u8_buf,
            SeekOffset::Start(amount) => {
                core::hint::cold_path();
                if amount >= pb_u8_buf.len() {
                    return Err(FSError::InvalidOffset);
                }

                &mut pb_u8_buf[amount..]
            }
            SeekOffset::End(amount) => {
                core::hint::cold_path();
                let actual_off = pb_u8_buf
                    .len()
                    .checked_sub(amount)
                    .ok_or(FSError::InvalidOffset)?;
                if actual_off >= pb_u8_buf.len() {
                    return Err(FSError::InvalidOffset);
                }

                &mut pb_u8_buf[actual_off..]
            }
        };

        let write_len = bytes.len().min(pb_u8_buf.len());

        pb_u8_buf[..write_len].copy_from_slice(&bytes[..write_len]);
        Ok(write_len)
    }

    pub fn sync_pixels_full(&mut self) {
        self.sync_pixels(0, self.info.stride * self.info.height);
    }

    pub fn sync_pixels_rect(&mut self, off_x: usize, off_y: usize, width: usize, height: usize) {
        let bytes_per_pixel = self.info.bytes_per_pixel;

        let pixels = &self.pixel_buffer
            [self.buffer_display_index..self.buffer_display_index + self.video_buffer.len() / 4];

        for row in 0..height {
            let start = off_x + ((off_y + row) * self.info.stride);
            let end = start + width;

            let pixels = &pixels[start..end];

            let start_byte = start * bytes_per_pixel;
            for (indx, pix) in pixels.iter().copied().enumerate() {
                let indx = start_byte + (indx * bytes_per_pixel);
                let pixel_bytes = pix.to_ne_bytes();

                self.video_buffer[indx + 0] = pixel_bytes[0];
                self.video_buffer[indx + 1] = pixel_bytes[1];
                self.video_buffer[indx + 2] = pixel_bytes[2];
            }
        }
    }

    /// Syncs pixel_count pixels in the buffer to the actual video_buffer starting at pixel_start
    pub fn sync_pixels(&mut self, pixel_start: usize, pixel_count: usize) {
        let width = self.info.stride;
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let pitch = width * bytes_per_pixel;

        let pixels = &self.pixel_buffer
            [self.buffer_display_index..self.buffer_display_index + self.video_buffer.len() / 4];

        let start = pixel_start.min(pixels.len() - 1);
        let end = (start + pixel_count).min(pixels.len());

        for i in start..end {
            let row = i / width;
            let row_start = row * pitch;

            let col = i % width;

            let indx = row_start + (col * bytes_per_pixel);

            let pixel = pixels[i];
            let pixel_bytes = pixel.to_ne_bytes();

            self.video_buffer[indx + 0] = pixel_bytes[0];
            self.video_buffer[indx + 1] = pixel_bytes[1];
            self.video_buffer[indx + 2] = pixel_bytes[2];
        }
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
                /* subtracting one screen from the buffer */
                let max_index = self.pixel_buffer.len() - (self.info.stride * self.info.height);
                let new_index = self.buffer_display_index + amount;

                if new_index <= max_index {
                    // We don't need to recopy
                    self.buffer_display_index = new_index;
                } else {
                    self.buffer_display_index = max_index;
                    self.pixel_buffer.copy_within(amount.., 0);
                }
            }
            core::cmp::Ordering::Equal => {}
        }

        self.sync_pixels_full();
    }

    #[inline(always)]
    /// sets the cursor to `pixel` in pixels
    pub fn set_cursor(&mut self, pixel: usize) {
        self.buffer_display_index = pixel;
    }

    pub fn fill(&mut self, color: RGB) {
        let color: u32 = color.into();
        self.pixel_buffer.fill(color);
    }
}
pub struct FrameBufferDriver {
    mapped_frames: Vec<Frame>,
    info: FrameBufferInfo,
    inner: Mutex<FrameBuffer<'static>>,
}

impl FrameBufferDriver {
    pub fn frames(&self) -> &[Frame] {
        &self.mapped_frames
    }

    pub fn create(pixel_buffers_count: usize) -> Self {
        let (video_buffer, info) = &*limine::LIMINE_FRAMEBUFFER;
        assert_eq!(info.bytes_per_pixel, 4);
        unsafe {
            let framebuffer = FrameBuffer::new(*video_buffer.get(), pixel_buffers_count, *info);

            let pb = &framebuffer.pixel_buffer;
            let ptr = pb.as_ptr();
            let virt_addr = VirtAddr::from_ptr(ptr);

            let len = pb.len() * 4;
            let page_n = len / PAGE_SIZE;

            let mut frames = Vec::with_capacity(page_n);
            for i in 0..page_n {
                let page_addr = virt_addr + i * PAGE_SIZE;
                let page = Page::containing_address(page_addr);
                let frame = current_higher_root_table()
                    .get_frame(page)
                    .expect("Failed to get Frame of a page belonging to a created double buffer");

                frames.push(frame);
            }
            Self {
                info: *info,
                inner: Mutex::new(framebuffer),
                mapped_frames: frames,
            }
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
    pub fn buffer<'s>(&'s self) -> MutexGuard<'s, FrameBuffer<'static>> {
        self.inner.lock()
    }
}

lazy_static! {
    pub static ref FRAMEBUFFER_DRIVER: FrameBufferDriver = FrameBufferDriver::create(4);
}
