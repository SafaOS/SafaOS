use core::ops::Add;

use crate::sync::IntSpinLock;
use alloc::{boxed::Box, collections::TryReserveError, vec::Vec};
use thiserror::Error;

use crate::{bootloader::PixelFormat, mem::vmm::VMMAlloc, oninit};

#[derive(Debug, Clone, Copy, Default)]
pub struct UPoint {
    x: u32,
    y: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Point {
    x: i32,
    y: i32,
}

impl Add<UPoint> for Point {
    type Output = Self;
    fn add(self, rhs: UPoint) -> Self::Output {
        Self {
            x: self.x.strict_add_unsigned(rhs.x),
            y: self.y.strict_add_unsigned(rhs.y),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    width: u32,
    height: u32,
}

#[inline]
fn interpret_cords(
    position_within: Point,
    bounds_within: Bounds,
    self_bounds: Bounds,
) -> (UPoint, UPoint, Bounds) {
    let p_w = position_within;
    let b_w = bounds_within;

    let x_neg = p_w.x.is_negative();
    let y_neg = p_w.y.is_negative();

    let x_abs = p_w.x.unsigned_abs();
    let y_abs = p_w.y.unsigned_abs();

    let x = if x_neg { 0 } else { x_abs };
    let y = if y_neg { 0 } else { y_abs };

    let start_col = if x_neg { x_abs } else { 0 };
    let start_row = if y_neg { y_abs } else { 0 };

    let act_width = if x_neg {
        b_w.width.saturating_sub(x_abs)
    } else {
        b_w.width
    };
    let act_height = if y_neg {
        b_w.height.saturating_sub(y_abs)
    } else {
        b_w.height
    };

    let width = act_width.min(self_bounds.width.saturating_sub(x));
    let height = act_height.min(self_bounds.height.saturating_sub(y));
    (
        UPoint { x, y },
        UPoint {
            x: start_col,
            y: start_row,
        },
        Bounds { width, height },
    )
}

#[inline(always)]
// AI generated
fn blit<F: FnMut(&[u8], &mut [u8])>(
    size: Bounds,
    src: &[u8],
    src_pitch: usize,
    src_origin: UPoint,
    src_bpp: usize,
    dst: &mut [u8],
    dst_pitch: usize,
    dst_origin: UPoint,
    dst_bpp: usize,
    mut row_fn: F,
) {
    let src_row_len = size.width as usize * src_bpp;
    let dst_row_len = size.width as usize * dst_bpp;

    for row in 0..size.height as usize {
        let s = (src_origin.y as usize + row) * src_pitch + src_origin.x as usize * src_bpp;
        let d = (dst_origin.y as usize + row) * dst_pitch + dst_origin.x as usize * dst_bpp;

        let (Some(s_row), Some(d_row)) =
            (src.get(s..s + src_row_len), dst.get_mut(d..d + dst_row_len))
        else {
            return;
        };

        row_fn(s_row, d_row);
    }
}

#[inline(always)]
fn swap_rb_row(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
        d[3] = s[3];
    }
}

#[inline(always)]
pub fn copy_pixels_to(
    in_bounds: Bounds,
    in_pitch: usize, // bytes per src row
    in_pixels: &[u8],
    in_format: PixelFormat,

    draw_bounds: Bounds,
    draw_position: Point,

    out_format: PixelFormat,
    out_bounds: Bounds,
    out_pitch: usize, // bytes per out row
    out_pixels: &mut [u8],
) {
    let (dst_origin, src_origin, clipped) = interpret_cords(draw_position, draw_bounds, out_bounds);

    let size = Bounds {
        width: clipped
            .width
            .min(in_bounds.width.saturating_sub(src_origin.x)),
        height: clipped
            .height
            .min(in_bounds.height.saturating_sub(src_origin.y)),
    };
    if size.width == 0 || size.height == 0 {
        return;
    }

    let in_bpp = in_format.bytes_per_pixel() as usize;
    let out_bpp = out_format.bytes_per_pixel() as usize;

    macro_rules! run {
        ($f:expr) => {
            blit(
                size, in_pixels, in_pitch, src_origin, in_bpp, out_pixels, out_pitch, dst_origin,
                out_bpp, $f,
            )
        };
    }

    match (in_format, out_format) {
        (PixelFormat::BgrX888, PixelFormat::BgrX888)
        | (PixelFormat::RgbX888, PixelFormat::RgbX888) => {
            run!(|s, d| d.copy_from_slice(s))
        }
        (PixelFormat::BgrX888, PixelFormat::RgbX888)
        | (PixelFormat::RgbX888, PixelFormat::BgrX888) => run!(swap_rb_row),
    }
}

#[derive(Debug)]
pub struct Framebuffer {
    bounds: Bounds,
    format: PixelFormat,
    pitch: usize,
    pixels: IntSpinLock<&'static mut [u8]>,
}
oninit::define! {
    unsafe static FRAMEBUFFER: Option<Framebuffer> = || crate::bootloader::framebuffer().map(|fb| {
        let count_bytes = fb.pitch * fb.height as usize ;

        let slice = unsafe { core::slice::from_raw_parts_mut(fb.base.cast::<u8>().as_mut(), count_bytes) };
        Framebuffer { bounds: Bounds { width: fb.width, height: fb.height }, format: fb.format, pitch: fb.pitch, pixels: IntSpinLock::new(slice) }
    });

}

oninit::define_routine! {
    pub unsafe fn INI_FRAMEBUFFER = with FRAMEBUFFER || {};
}

/// Returns framebuffer width.
pub fn width() -> u32 {
    FRAMEBUFFER
        .as_ref()
        .map(|fb| fb.bounds.width)
        .unwrap_or_default()
}

pub fn height() -> u32 {
    FRAMEBUFFER
        .as_ref()
        .map(|fb| fb.bounds.height)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct FbSurface {
    position: Point,
    bounds: Bounds,
    format: PixelFormat,
    damage_min: UPoint,
    damage_max: UPoint,
    pixels: Box<[u8], VMMAlloc>,
}

#[derive(Debug, Clone, Error)]
pub enum SurfaceError {
    #[error("No framebuffer found to base the surface on")]
    NoDisplay,
    #[error("Failed to allocate memory for a framebuffer: {0}")]
    FailedToReserveMemory(#[from] TryReserveError),
}

impl FbSurface {
    pub fn damage(&mut self) {
        self.damage_min = UPoint { x: 0, y: 0 };

        self.damage_max = UPoint {
            x: self.width(),
            y: self.height(),
        };
    }

    #[inline]
    /// Shifts up by `pixels` pixels.
    pub fn shift_up(&mut self, pixels: u32) {
        let pixels = pixels as usize * self.width() as usize * self.bpp();
        if pixels == 0 {
            return;
        }

        let len = self.pixels.len();
        assert!(len >= pixels, "shift up by too many pixels");

        self.pixels.copy_within(pixels.., 0);
        self.damage();
    }

    #[inline]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    #[inline]
    /// Returns bytes per pixel
    pub const fn bpp(&self) -> usize {
        self.format.bytes_per_pixel() as usize
    }

    #[inline]
    pub const fn width(&self) -> u32 {
        self.bounds.width
    }

    #[inline]
    pub const fn height(&self) -> u32 {
        self.bounds.height
    }

    /// Creates a framebuffer safe that can be drawn to.
    pub fn create_with_fb_base(
        fb_x: i32,
        fb_y: i32,
        width: u32,
        height: u32,
    ) -> Result<Self, SurfaceError> {
        let Some(format) = FRAMEBUFFER.as_ref().map(|fb| fb.format) else {
            return Err(SurfaceError::NoDisplay);
        };

        let size = width as usize * height as usize * format.bytes_per_pixel() as usize;
        Vec::try_with_capacity_in(size, VMMAlloc::new(&"FbSurface"))
            .map(|mut mem| {
                mem.resize(size, 0);

                Self {
                    position: Point { x: fb_x, y: fb_y },
                    bounds: Bounds { width, height },
                    format,
                    damage_min: UPoint { x: 0, y: 0 },
                    damage_max: UPoint {
                        x: width,
                        y: height,
                    },
                    pixels: mem.into_boxed_slice(),
                }
            })
            .map_err(|e| e.into())
    }

    #[inline]
    pub fn fill(&mut self, x: u32, y: u32, width: u32, height: u32, pixel: &[u8]) {
        let bpp = self.format.bytes_per_pixel() as usize;
        assert_eq!(bpp, pixel.len());

        for y in y..(y + height).min(self.bounds.height) {
            let pix_idx = (self.bounds.width as usize * y as usize) + x as usize;
            let row = &mut self.pixels[pix_idx * bpp..(pix_idx + width as usize) * bpp];

            for chunk in row.chunks_exact_mut(bpp) {
                chunk.copy_from_slice(pixel);
            }
        }
    }

    /// Sets pixels from x to y, to the following `pixels` bytes.
    #[inline]
    pub fn set_pixels(&mut self, x: u32, y: u32, pixels: &[u8]) {
        let bpp = self.format.bytes_per_pixel() as usize;
        if bpp == 0 || x >= self.bounds.width || y >= self.bounds.height {
            return;
        }

        let count = (pixels.len() / bpp).min((self.bounds.width - x) as usize);
        if count == 0 {
            return;
        }

        let pitch = self.bounds.width as usize * bpp;
        let start = y as usize * pitch + x as usize * bpp;
        let len = count * bpp;

        let Some(dst) = self.pixels.get_mut(start..start + len) else {
            return;
        };
        dst.copy_from_slice(&pixels[..len]);

        let end_x = x + count as u32;
        self.damage_min.x = self.damage_min.x.min(x);
        self.damage_min.y = self.damage_min.y.min(y);
        self.damage_max.x = self.damage_max.x.max(end_x);
        self.damage_max.y = self.damage_max.y.max(y + 1);
    }

    /// Draws surface change.
    pub fn display(&mut self) {
        let Some(fb) = &*FRAMEBUFFER else {
            return;
        };

        let draw_bounds = Bounds {
            width: self.damage_max.x - self.damage_min.x,
            height: self.damage_max.y - self.damage_min.y,
        };

        if draw_bounds.width == 0 || draw_bounds.height == 0 {
            return;
        }

        let mut out_pixels = fb.pixels.lock();

        copy_pixels_to(
            self.bounds,
            self.bounds.width as usize * self.format.bytes_per_pixel() as usize,
            &self.pixels,
            self.format,
            draw_bounds,
            self.position + self.damage_min,
            fb.format,
            fb.bounds,
            fb.pitch,
            *out_pixels,
        );

        self.damage_min = UPoint::default();
        self.damage_max = UPoint::default();
    }
}
