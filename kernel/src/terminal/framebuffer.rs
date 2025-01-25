const RASTER_HEIGHT: RasterHeight = RasterHeight::Size20;
const FONT_WEIGHT: FontWeight = FontWeight::Regular;
const RASTER_WIDTH: usize = get_raster_width(FONT_WEIGHT, RASTER_HEIGHT);

use core::fmt::Write;

use crate::utils::{
    ansi::{self, AnsiSequence},
    either::Either,
};
use noto_sans_mono_bitmap::{
    get_raster, get_raster_width, FontWeight, RasterHeight, RasterizedChar,
};
use spin::RwLock;

use super::TTYInterface;
use crate::{
    drivers::framebuffer::{FrameBuffer, FRAMEBUFFER_DRIVER},
    utils::display::RGB,
};

const DEFAULT_FG_COLOR: RGB = RGB::WHITE;
const DEFAULT_BG_COLOR: RGB = RGB::BLACK;

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
    pub fn new() -> Self {
        Self {
            framebuffer: &FRAMEBUFFER_DRIVER,
            cursor_x: 0,
            cursor_y: 0,
            fg_color: DEFAULT_FG_COLOR,
            bg_color: DEFAULT_BG_COLOR,
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

    fn draw_raster(&mut self, raster: RasterizedChar, fg_color: RGB, bg_color: RGB) {
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
                let color = fg_color.with_alpha(*byte, bg_color);

                framebuffer.set_pixel(x + col, y + row, color);
            }
        }

        self.cursor_x += 1;
    }

    fn remove_char(&mut self) {
        if self.cursor_x == 0 && self.cursor_y > 0 {
            self.cursor_x = (self.framebuffer.read().width() / RASTER_WIDTH) - 1;
            self.cursor_y -= 1;
        } else if self.cursor_x > 0 {
            self.cursor_x -= 1;
        }

        let mut framebuffer = self.framebuffer.write();
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
            self.fg_color = DEFAULT_FG_COLOR;
            self.bg_color = DEFAULT_BG_COLOR;
            return;
        }
        let mut params = params.iter().copied();

        while let Some(param) = params.next() {
            match param {
                0 => {
                    self.fg_color = DEFAULT_FG_COLOR;
                    self.bg_color = DEFAULT_BG_COLOR;
                }

                // 30-37 foreground colors
                30 => self.fg_color = RGB::BLACK,
                31 => self.fg_color = RGB::RED,
                32 => self.fg_color = RGB::GREEN,
                33 => self.fg_color = RGB::YELLOW,
                34 => self.fg_color = RGB::BLUE,
                35 => self.fg_color = RGB::MAGENTA,
                36 => self.fg_color = RGB::CYAN,
                37 => self.fg_color = RGB::WHITE,

                // 90-97 bright foreground colors
                90 => self.fg_color = RGB::BRIGHT_BLACK,
                91 => self.fg_color = RGB::BRIGHT_RED,
                92 => self.fg_color = RGB::BRIGHT_GREEN,
                93 => self.fg_color = RGB::BRIGHT_YELLOW,
                94 => self.fg_color = RGB::BRIGHT_BLUE,
                95 => self.fg_color = RGB::BRIGHT_MAGENTA,
                96 => self.fg_color = RGB::BRIGHT_CYAN,
                97 => self.fg_color = RGB::BRIGHT_WHITE,

                // 40-47 background colors
                40 => self.bg_color = RGB::BLACK,
                41 => self.bg_color = RGB::RED,
                42 => self.bg_color = RGB::GREEN,
                43 => self.bg_color = RGB::YELLOW,
                44 => self.bg_color = RGB::BLUE,
                45 => self.bg_color = RGB::MAGENTA,
                46 => self.bg_color = RGB::CYAN,
                47 => self.bg_color = RGB::WHITE,

                // 100-107 bright background colors
                100 => self.bg_color = RGB::BRIGHT_BLACK,
                101 => self.bg_color = RGB::BRIGHT_RED,
                102 => self.bg_color = RGB::BRIGHT_GREEN,
                103 => self.bg_color = RGB::BRIGHT_YELLOW,
                104 => self.bg_color = RGB::BRIGHT_BLUE,
                105 => self.bg_color = RGB::BRIGHT_MAGENTA,
                106 => self.bg_color = RGB::BRIGHT_CYAN,
                107 => self.bg_color = RGB::BRIGHT_WHITE,

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

            AnsiSequence::CursorUp(count) => self.offset_cursor(0, -(count as isize)),
            AnsiSequence::CursorDown(count) => self.offset_cursor(0, count as isize),
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
