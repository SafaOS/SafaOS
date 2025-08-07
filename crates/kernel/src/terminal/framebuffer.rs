const RASTER_HEIGHT: RasterHeight = RasterHeight::Size20;
const FONT_WEIGHT: FontWeight = FontWeight::Regular;
const RASTER_WIDTH: usize = get_raster_width(FONT_WEIGHT, RASTER_HEIGHT);
const CURSOR_CHAR: char = '_';
const TAB_WIDTH: usize = 5;

use crate::{
    drivers::framebuffer::FrameBufferDriver,
    utils::{
        ansi::{self, AnsiSequence},
        bstr::BStr,
        either::Either,
    },
};
use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};

use super::TTYInterface;
use crate::{drivers::framebuffer::FRAMEBUFFER_DRIVER, utils::display::RGB};

pub const DEFAULT_CURSOR_X: usize = 1;
pub const DEFAULT_CURSOR_Y: usize = 1;

pub struct FrameBufferTTY<'a> {
    framebuffer: &'a FrameBufferDriver,
    /// x position in characters
    cursor_x: usize,
    /// y position in characters
    cursor_y: usize,
    fg_color: RGB,
    bg_color: RGB,
    show_cursor: bool,
}

impl FrameBufferTTY<'_> {
    pub fn new() -> Self {
        let framebuffer = &FRAMEBUFFER_DRIVER;
        framebuffer.buffer().fill(RGB::BG_COLOR);
        framebuffer.buffer().sync_pixels_full();

        Self {
            framebuffer,
            cursor_x: DEFAULT_CURSOR_X,
            cursor_y: DEFAULT_CURSOR_Y,
            fg_color: RGB::FG_COLOR,
            bg_color: RGB::BG_COLOR,
            show_cursor: true,
        }
    }
    #[inline(always)]
    fn get_pixel_offset_at(&self) -> (usize, usize) {
        (self.get_x(), self.get_y())
    }

    #[inline(always)]
    fn get_current_pixel(&self) -> usize {
        let (x, y) = self.get_pixel_offset_at();
        (y * self.framebuffer.width()) + x
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

    #[inline(always)]
    fn check_draw_raster(&mut self, raster: &RasterizedChar) {
        let stride = self.framebuffer.width();
        let height = self.framebuffer.height();

        if self.get_x() + raster.width() * 2 > stride {
            self.newline();
        }

        if self.get_y() + raster.height() * 2 >= height {
            self.scroll_down();
            self.cursor_y -= 1;
        }
    }

    // TODO: refactor all the draw and remove functions
    // Draws a single character taking the specified position as a hint (in characters)
    // returns the actual position of the drawn character (in pixels)
    fn draw_raster_opaque(
        &mut self,
        raster: RasterizedChar,
        fg_color: RGB,
        bg_color: RGB,
        x: usize,
        y: usize,
    ) -> (usize, usize) {
        let stride = self.framebuffer.width();

        let (x, y) = if (x * RASTER_WIDTH) + raster.width() * 2 > stride {
            (
                DEFAULT_CURSOR_X * RASTER_WIDTH,
                (y + 1) * RASTER_HEIGHT.val(),
            )
        } else {
            (x * RASTER_WIDTH, y * RASTER_HEIGHT.val())
        };

        let mut buffer = self.framebuffer.buffer();

        for (row, rows) in raster.raster().iter().enumerate() {
            for (col, byte) in rows.iter().enumerate() {
                let color = fg_color.with_alpha(*byte, bg_color);
                if color != bg_color {
                    buffer.set_pixel(x + col, y + row, color);
                }
            }
        }
        (x, y)
    }

    fn draw_raster(&mut self, raster: RasterizedChar, fg_color: RGB, bg_color: RGB) {
        self.check_draw_raster(&raster);

        let (x, y) = self.get_pixel_offset_at();
        let mut buffer = self.framebuffer.buffer();

        for (row, rows) in raster.raster().iter().enumerate() {
            for (col, byte) in rows.iter().enumerate() {
                let color = fg_color.with_alpha(*byte, bg_color);
                buffer.set_pixel(x + col, y + row, color);
            }
        }

        self.cursor_x += 1;
    }

    fn remove_char_opaque(&mut self, c: char, bg_color: RGB, x: usize, y: usize) {
        let raster = self.raster(c);
        let (x, y) = (x * RASTER_WIDTH, y * RASTER_HEIGHT.val());

        let mut buffer = self.framebuffer.buffer();

        for (row, rows) in raster.raster().iter().enumerate() {
            for (col, byte) in rows.iter().enumerate() {
                if *byte != 0 {
                    buffer.set_pixel(x + col, y + row, bg_color);
                }
            }
        }
    }

    fn sync_pixels_full(&mut self) {
        self.framebuffer.buffer().sync_pixels_full();
    }

    fn sync_pixels_partial(&mut self, pixel0: usize, pixel1: usize) {
        if pixel0 == pixel1 {
            return;
        }

        let (start_pixel, end_pixel) = if pixel0 <= pixel1 {
            (pixel0, pixel1)
        } else {
            (pixel1, pixel0)
        };

        self.framebuffer
            .buffer()
            .sync_pixels(start_pixel, end_pixel - start_pixel);
    }

    fn putc_unsynced(&mut self, c: char) {
        let raster = self.raster(c);
        match c {
            '\n' => self.newline(),
            '\r' => {
                self.cursor_x = DEFAULT_CURSOR_X;
            }
            '\t' => {
                for _ in 0..TAB_WIDTH {
                    self.putc_unsynced(' ');
                }
            }
            _ => self.draw_raster(raster, self.fg_color, self.bg_color),
        }
    }

    fn handle_set_graphics_mode(&mut self, params: &[u8]) {
        if params.is_empty() {
            self.fg_color = RGB::FG_COLOR;
            self.bg_color = RGB::BG_COLOR;
            return;
        }
        let mut params = params.iter().copied();

        while let Some(param) = params.next() {
            match param {
                0 => {
                    self.fg_color = RGB::FG_COLOR;
                    self.bg_color = RGB::BG_COLOR;
                }

                39 => self.fg_color = RGB::FG_COLOR,
                49 => self.bg_color = RGB::BG_COLOR,

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

    fn write_str_unsynced(&mut self, s: &BStr) {
        ansi::AnsiiParser::new(s).for_each(|output| match output {
            Either::Left(escape) => self.handle_escape_sequence(escape),
            Either::Right(text) => {
                for chunk in text.utf8_chunks() {
                    let valid = chunk.valid();
                    for c in valid.chars() {
                        self.putc_unsynced(c);
                    }
                    if !chunk.invalid().is_empty() {
                        self.putc_unsynced(char::REPLACEMENT_CHARACTER);
                    }
                }
            }
        });
    }
}

impl TTYInterface for FrameBufferTTY<'_> {
    fn write_str(&mut self, s: &BStr) {
        let pixel0 = self.get_current_pixel();
        self.write_str_unsynced(s);
        let pixel1 = self.get_current_pixel();

        self.sync_pixels_partial(pixel0, pixel1);
    }

    fn newline(&mut self) {
        self.cursor_x = DEFAULT_CURSOR_X;
        self.cursor_y += 1;
    }

    fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor_x = x;
        self.cursor_y = y;
    }

    fn offset_cursor(&mut self, x: isize, y: isize) {
        let max_x = (self.framebuffer.width() / (RASTER_WIDTH)) - (DEFAULT_CURSOR_X * 2);
        let max_y = (self.framebuffer.height() / (RASTER_HEIGHT.val())) - (DEFAULT_CURSOR_Y * 2);

        let mut y = self.cursor_y.checked_add_signed(y).map(|y| y.min(max_y));
        let x = self.cursor_x.checked_add_signed(x).map(|x| x.min(max_x));

        match x {
            Some(x) if x >= DEFAULT_CURSOR_X => {
                self.cursor_x = x;
            }
            _ => {
                y = y.map(|y| y.saturating_sub(1));
                self.cursor_x = max_x;
            }
        }

        match y {
            Some(y) if y >= DEFAULT_CURSOR_Y => {
                self.cursor_y = y;
            }
            _ => {
                self.cursor_x = DEFAULT_CURSOR_X;
                self.cursor_y = DEFAULT_CURSOR_Y;
            }
        }
    }

    fn scroll_down(&mut self) {
        let stride = self.framebuffer.width() * RASTER_HEIGHT.val();
        let mut framebuffer = self.framebuffer.buffer();

        framebuffer.shift_buffer(stride as isize);
    }

    fn scroll_up(&mut self) {
        let stride = self.framebuffer.width() * RASTER_HEIGHT.val();
        let mut framebuffer = self.framebuffer.buffer();
        framebuffer.shift_buffer(-(stride as isize));
    }

    fn clear(&mut self) {
        let mut buffer = self.framebuffer.buffer();
        buffer.fill(self.bg_color);
        buffer.set_cursor(0);
        drop(buffer);

        self.cursor_y = 0;
        self.sync_pixels_full();
    }

    fn hide_cursor(&mut self) {
        if self.show_cursor {
            let pixel0 = self.get_current_pixel();
            self.remove_char_opaque(CURSOR_CHAR, self.bg_color, self.cursor_x, self.cursor_y);
            // Since it draws at the current position, we need to sync all pixels from the current position to the position draw raster opaque drawd at + a single character
            let pixel1 = pixel0 + (RASTER_WIDTH * RASTER_HEIGHT.val());

            self.sync_pixels_partial(pixel0, pixel1);

            self.show_cursor = false;
        }
    }
    fn draw_cursor(&mut self) {
        let raster = self.raster(CURSOR_CHAR);

        let (x, y) = self.draw_raster_opaque(
            raster,
            RGB::WHITE,
            self.bg_color,
            self.cursor_x,
            self.cursor_y,
        );
        let pixel0 = self.get_current_pixel();
        // Syncs all pixels from the current position to the position draw raster opaque drawd at + a single character
        let pixel1 =
            pixel0 + (x + y * self.framebuffer.width()) + (RASTER_WIDTH * RASTER_HEIGHT.val());

        self.sync_pixels_partial(pixel0, pixel1);
        self.show_cursor = true;
    }
}
