const RASTER_HEIGHT: RasterHeight = RasterHeight::Size20;
const FONT_WEIGHT: FontWeight = FontWeight::Regular;
const RASTER_WIDTH: usize = get_raster_width(FONT_WEIGHT, RASTER_HEIGHT);

use core::fmt::Write;

use crate::utils::{
    ansi::{self, AnsiSequence},
    either::Either,
};
use lazy_static::lazy_static;
use noto_sans_mono_bitmap::{
    get_raster, get_raster_width, FontWeight, RasterHeight, RasterizedChar,
};
use spin::RwLock;

use super::TTYInterface;
use crate::{
    drivers::framebuffer::{FrameBuffer, FRAMEBUFFER_DRIVER},
    utils::{
        display::{BLACK, BLUE, CYAN, GRAY, GREEN, MAGENTA, RED, RGB, WHITE, YELLOW},
        Locked,
    },
};

pub struct FrameBufferTTY<'a> {
    framebuffer: &'a RwLock<FrameBuffer>,
    /// x position in characters
    cursor_x: usize,
    /// y position in characters
    cursor_y: usize,
    fg_color: RGB,
    bg_color: RGB,
}

impl FrameBufferTTY<'_> {
    fn new() -> Self {
        let size_pixels = FRAMEBUFFER_DRIVER.read().width() * FRAMEBUFFER_DRIVER.read().height();
        let bytes_per_pixel = FRAMEBUFFER_DRIVER.read().info.bytes_per_pixel;
        let size = size_pixels * bytes_per_pixel;

        FRAMEBUFFER_DRIVER.write().increase_buffer(size * 3);
        Self {
            framebuffer: &FRAMEBUFFER_DRIVER,
            cursor_x: 0,
            cursor_y: 0,
            fg_color: RGB::new(255, 255, 255),
            bg_color: RGB::new(0, 0, 0),
        }
    }
    #[inline(always)]
    fn get_pixel_at(&self) -> (usize, usize) {
        (self.get_x(), self.get_y())
    }
    #[inline(always)]
    fn get_x(&self) -> usize {
        self.cursor_x * RASTER_WIDTH
    }
    #[inline(always)]
    fn get_y(&self) -> usize {
        self.cursor_y * RASTER_HEIGHT.val()
    }

    fn raster(&self, c: char) -> RasterizedChar {
        get_raster(c, FONT_WEIGHT, RASTER_HEIGHT).unwrap_or(
            get_raster('?', FONT_WEIGHT, RASTER_HEIGHT).expect("failed to get rasterized char"),
        )
    }

    fn draw_raster(&mut self, raster: RasterizedChar, fg_color: RGB, _bg_color: RGB) {
        let framebuffer = self.framebuffer.read();
        let stride = framebuffer.info.stride;
        let cursor = framebuffer.get_cursor();
        let height = framebuffer.height();
        drop(framebuffer);

        if self.get_x() + raster.width() > stride {
            self.newline();
        }

        if self.get_y() + raster.height() >= cursor / stride + height {
            self.scroll_down();
        }

        let (x, y) = self.get_pixel_at();
        let mut framebuffer = self.framebuffer.write();

        for (row, rows) in raster.raster().iter().enumerate() {
            for (col, byte) in rows.iter().enumerate() {
                let (red, green, blue) = fg_color.tuple();
                let (red, green, blue) = (
                    red * (*byte > 0) as u8,
                    green * (*byte > 0) as u8,
                    blue * (*byte > 0) as u8,
                );
                let fg_color = RGB::new(red, green, blue);

                framebuffer.set_pixel(x + col, y + row, fg_color);
            }
        }

        self.cursor_x += 1;
    }

    fn remove_char(&mut self) {
        let mut framebuffer = self.framebuffer.write();
        self.cursor_x -= 1;
        let (x, y) = self.get_pixel_at();

        for row in 0..RASTER_HEIGHT.val() {
            for col in 0..RASTER_WIDTH {
                framebuffer.set_pixel(x + col, y + row, RGB::new(0, 0, 0));
            }
        }
    }

    fn sync_pixels(&mut self) {
        self.framebuffer.write().sync_pixels();
    }

    fn putc_unsynced(&mut self, c: char) {
        let raster = self.raster(c);
        match c {
            '\n' => self.newline(),
            '\r' => self.cursor_x = 0,
            _ => self.draw_raster(raster, self.fg_color, self.bg_color),
        }
    }

    fn handle_set_graphics_mode(&mut self, params: &[u8]) {
        if params.is_empty() {
            self.fg_color = RGB::new(255, 255, 255);
            self.bg_color = RGB::new(0, 0, 0);
            return;
        }
        let mut params = params.iter().copied();

        while let Some(param) = params.next() {
            match param {
                0 => {
                    self.fg_color = RGB::new(255, 255, 255);
                    self.bg_color = RGB::new(0, 0, 0);
                    return;
                }

                30 => self.fg_color = BLACK,
                31 => self.fg_color = RED,
                32 => self.fg_color = GREEN,
                33 => self.fg_color = YELLOW,
                34 => self.fg_color = BLUE,
                35 => self.fg_color = MAGENTA,
                36 => self.fg_color = CYAN,
                37 => self.fg_color = WHITE,
                90 => self.fg_color = GRAY,

                40 => self.bg_color = BLACK,
                41 => self.bg_color = RED,
                42 => self.bg_color = GREEN,
                43 => self.bg_color = YELLOW,
                44 => self.bg_color = BLUE,
                45 => self.bg_color = MAGENTA,
                46 => self.bg_color = CYAN,
                47 => self.bg_color = WHITE,

                38 => {
                    if Some(2) == params.next() {
                        let red = params.next().unwrap_or_default();
                        let green = params.next().unwrap_or_default();
                        let blue = params.next().unwrap_or_default();

                        self.fg_color = RGB::new(red, green, blue);
                    }
                }

                48 => {
                    if Some(2) == params.next() {
                        let red = params.next().unwrap_or_default();
                        let green = params.next().unwrap_or_default();
                        let blue = params.next().unwrap_or_default();

                        self.bg_color = RGB::new(red, green, blue);
                    }
                }
                _ => (),
            }
        }
    }

    fn handle_escape_sequence(&mut self, escape: AnsiSequence) {
        match escape {
            AnsiSequence::SetGraphicsMode(params) => {
                self.handle_set_graphics_mode(&params);
            }

            AnsiSequence::CursorUp(count) => self.offset_cursor(0, count as isize),
            AnsiSequence::CursorDown(count) => self.offset_cursor(0, -(count as isize)),
            AnsiSequence::CursorForward(count) => self.offset_cursor(count as isize, 0),
            AnsiSequence::CursorBackward(count) => self.offset_cursor(-(count as isize), 0),
            AnsiSequence::CursorPos(x, y) => self.set_cursor(x as usize, y as usize),

            AnsiSequence::EraseDisplay => self.clear(),
        }
    }

    fn write_str_unsynced(&mut self, s: &str) {
        ansi::AnsiiParser::new(s).for_each(|output| match output {
            Either::Left(escape) => self.handle_escape_sequence(escape),
            Either::Right(text) => {
                for c in text.chars() {
                    self.putc_unsynced(c);
                }
            }
        });
    }
}

impl Write for FrameBufferTTY<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str_unsynced(s);
        self.sync_pixels();
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.putc_unsynced(c);
        self.sync_pixels();
        Ok(())
    }
}

impl TTYInterface for FrameBufferTTY<'_> {
    fn newline(&mut self) {
        self.cursor_x = 0;
        self.cursor_y += 1;
    }

    fn backspace(&mut self) {
        self.remove_char();
        self.sync_pixels();
    }

    fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor_x = x;
        self.cursor_y = y;
    }

    fn offset_cursor(&mut self, x: isize, y: isize) {
        self.cursor_x = (self.cursor_x as isize + x) as usize;
        self.cursor_y = (self.cursor_y as isize + y) as usize;
    }

    fn scroll_down(&mut self) {
        let mut framebuffer = self.framebuffer.write();
        let stride = framebuffer.info.stride * RASTER_HEIGHT.val();
        framebuffer.shift_buffer(stride as isize);
    }

    fn scroll_up(&mut self) {
        let mut framebuffer = self.framebuffer.write();
        let stride = framebuffer.info.stride * RASTER_HEIGHT.val();
        framebuffer.shift_buffer(-(stride as isize));
    }

    fn clear(&mut self) {
        let stride = self.framebuffer.read().info.stride;
        self.framebuffer.write().clear();

        let old_cursor = self.framebuffer.read().get_cursor();
        self.framebuffer.write().set_cursor(0);

        let diff = old_cursor / stride / RASTER_HEIGHT.val();
        self.cursor_y -= diff;

        self.sync_pixels();
    }
}

lazy_static! {
    pub static ref FRAMEBUFFER_TTY_INTERFACE: Locked<FrameBufferTTY<'static>> =
        Locked::new(FrameBufferTTY::new());
}
