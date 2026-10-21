use core::fmt::{Arguments, Write};

use crate::{
    bootloader::PixelFormat,
    fb::{self, FbSurface, INI_FRAMEBUFFER},
    logging,
    mem::alloc::INI_ALLOC,
    oninit,
    sync::SpinLockIrq,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RGB {
    blue: u8,
    green: u8,
    red: u8,
    alpha: u8,
}

pub const BLACK: RGB = RGB::hex(0x282828);
pub const BRIGHT_BLACK: RGB = RGB::hex(0x928374);

pub const WHITE: RGB = RGB::hex(0xa89984);
pub const BRIGHT_WHITE: RGB = RGB::hex(0xebdbb2);

pub const RED: RGB = RGB::hex(0xcc241d);
pub const BRIGHT_RED: RGB = RGB::hex(0xfb4934);

pub const GREEN: RGB = RGB::hex(0x98971a);
pub const BRIGHT_GREEN: RGB = RGB::hex(0xb8bb26);

pub const BLUE: RGB = RGB::hex(0x458588);
pub const BRIGHT_BLUE: RGB = RGB::hex(0x83a598);

pub const YELLOW: RGB = RGB::hex(0xd79921);
pub const BRIGHT_YELLOW: RGB = RGB::hex(0xfabd2f);

pub const CYAN: RGB = RGB::hex(0x689d6a);
pub const BRIGHT_CYAN: RGB = RGB::hex(0x8ec07c);

pub const MAGENTA: RGB = RGB::hex(0xb16286);
pub const BRIGHT_MAGENTA: RGB = RGB::hex(0xd3869b);
impl RGB {
    pub const BG_COLOR: RGB = BLACK;
    pub const FG_COLOR: RGB = BRIGHT_WHITE;

    #[inline(always)]
    pub const fn as_format(self, fmt: PixelFormat) -> [u8; 4] {
        match fmt {
            PixelFormat::BgrX888 => [self.b(), self.g(), self.r(), 0],
            PixelFormat::RgbX888 => [self.r(), self.g(), self.b(), 0],
        }
    }

    pub const fn r(&self) -> u8 {
        self.red
    }

    pub const fn g(&self) -> u8 {
        self.green
    }

    pub const fn b(&self) -> u8 {
        self.blue
    }

    #[inline]
    /// Applies alpha to the pixel
    pub const fn with_alpha(self, alpha: u8) -> Self {
        let alpha = alpha as u16;
        let red = self.red as u16;
        let green = self.green as u16;
        let blue = self.blue as u16;

        Self {
            blue: (blue * alpha / 255) as u8,
            green: (green * alpha / 255) as u8,
            red: (red * alpha / 255) as u8,
            alpha: alpha as u8,
        }
    }

    /// Constructs a pixel from RGB values
    #[inline(always)]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            blue: b,
            green: g,
            red: r,
            alpha: 255,
        }
    }

    #[inline(always)]
    /// Construct a Pixel from a hex RGB Color
    pub const fn hex(rgb: u32) -> Self {
        let red = (rgb >> 16) as u8;
        let green = (rgb >> 8) as u8;
        let blue = rgb as u8;

        Self::rgb(red, green, blue)
    }

    #[inline(always)]
    /// Alpha blends a pixel with another
    pub const fn blend(&self, bottom: &Self) -> Self {
        if self.alpha == 0 {
            return *bottom;
        }

        if bottom.alpha == 0 || self.alpha == 0xFF {
            return *self;
        }

        let top_red = self.red as u16;
        let bottom_red = bottom.red as u16;

        let top_green = self.green as u16;
        let bottom_green = bottom.green as u16;

        let top_blue = self.blue as u16;
        let bottom_blue = bottom.blue as u16;

        let top_alpha = self.alpha as u16;
        let bottom_alpha = bottom.alpha as u16;

        #[inline(always)]
        const fn calc_color(top: u16, bottom: u16, alpha: u16) -> u16 {
            top + (bottom * (alpha ^ 0xFF) / 255)
        }

        let red = calc_color(top_red, bottom_red, top_alpha);
        let green = calc_color(top_green, bottom_green, top_alpha);
        let blue = calc_color(top_blue, bottom_blue, top_alpha);
        let alpha = calc_color(top_alpha, bottom_alpha, top_alpha);

        Self {
            red: red as u8,
            green: green as u8,
            blue: blue as u8,
            alpha: alpha as u8,
        }
    }
}

const CHAR_WIDTH: u32 = noto_sans_mono_bitmap::get_raster_width(
    noto_sans_mono_bitmap::FontWeight::Regular,
    noto_sans_mono_bitmap::RasterHeight::Size20,
) as u32;

const CHAR_HEIGHT: u32 = noto_sans_mono_bitmap::RasterHeight::Size20.val() as u32;

#[derive(Debug)]
pub struct Terminal {
    surface: FbSurface,
    bg_color: RGB,
    fg_color: RGB,

    col: u32,
    line: u32,
}

const UNKNOWN_RASTER: noto_sans_mono_bitmap::RasterizedChar = noto_sans_mono_bitmap::get_raster(
    '�',
    noto_sans_mono_bitmap::FontWeight::Regular,
    noto_sans_mono_bitmap::RasterHeight::Size20,
)
.expect("No unknown character raster found");

impl Terminal {
    #[inline(always)]
    fn render_char_at(&mut self, x: u32, y: u32, c: char) {
        let raster = noto_sans_mono_bitmap::get_raster(
            c,
            noto_sans_mono_bitmap::FontWeight::Regular,
            noto_sans_mono_bitmap::RasterHeight::Size20,
        )
        .unwrap_or(UNKNOWN_RASTER);

        let bitmap = raster.raster();
        for (row_i, row) in bitmap.iter().enumerate().take(CHAR_HEIGHT as usize) {
            for (col_i, col) in row.iter().enumerate() {
                let pixel = self
                    .fg_color
                    .with_alpha(*col)
                    .blend(&self.bg_color)
                    .as_format(self.surface.format());
                // FIXME: set the surface format to RGBA32 instead of doing all of this
                let pixel = &pixel[..self.surface.format().bytes_per_pixel() as usize];

                self.surface
                    .set_pixels(x + col_i as u32, y + row_i as u32, pixel);
            }
        }
    }

    fn normalize_cursor_position(&mut self) {
        let max_col = self.surface.width() / CHAR_WIDTH;
        if self.col >= max_col {
            let lines_diff = self.col / max_col;

            self.line += lines_diff;
            self.col = 0;
        }

        let max_line = self.surface.height() / CHAR_HEIGHT;
        if self.line >= max_line {
            let lines_diff = (self.line - max_line) + 1;

            self.surface
                .shift_up((lines_diff * CHAR_HEIGHT).min(self.surface.height()));

            self.line -= lines_diff;

            let pixel = self.bg_color.as_format(self.surface.format());
            self.surface.fill(
                0,
                self.line * CHAR_HEIGHT,
                max_col * CHAR_WIDTH,
                (max_line - self.line) * CHAR_HEIGHT,
                &pixel[..self.surface.bpp()],
            );
        }
    }

    #[inline]
    pub fn add_char(&mut self, c: char) {
        match c {
            '\n' => {
                self.line += 1;
                self.col = 0;
            }
            o => {
                self.normalize_cursor_position();
                self.render_char_at(self.col * CHAR_WIDTH, self.line * CHAR_HEIGHT, o);
                self.col += 1;
            }
        }
    }

    #[inline]
    /// Updates the screen (displays rendered results)
    pub fn display(&mut self) {
        self.surface.display();
    }
}

oninit::define! {
    unsafe static TERMINAL_EMU: Option<SpinLockIrq<Terminal>> = with INI_ALLOC, INI_FRAMEBUFFER || 'blk: {
        let Ok(mut surface) = FbSurface::create_with_fb_base(0, 0, fb::width(), fb::height()) else {
            break 'blk None;
        };

        crate::logging::debug!(Terminal, "Surface: {}x{} {:?}", surface.width(), surface.height(), surface.format());
        surface.fill(0, 0, surface.width(), surface.height(), &RGB::BG_COLOR.as_format(surface.format())[..surface.format().bytes_per_pixel() as usize]);
        Some(SpinLockIrq::new(Terminal {
            surface,
            line:0,
            col:0,
            fg_color: RGB::FG_COLOR,
            bg_color: RGB::BG_COLOR,
        }))
    };
}

impl Write for Terminal {
    #[inline]
    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.add_char(c);
        self.display();
        Ok(())
    }

    #[inline]
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        vte::Parser::new().advance(self, s.as_bytes());
        self.display();
        Ok(())
    }

    fn write_fmt(&mut self, args: Arguments<'_>) -> core::fmt::Result {
        struct FakeWriter<'a> {
            parser: vte::Parser,
            term: &'a mut Terminal,
        }

        impl<'a> core::fmt::Write for FakeWriter<'a> {
            #[inline]
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                self.parser.advance(self.term, s.as_bytes());
                Ok(())
            }
        }

        FakeWriter {
            parser: vte::Parser::new(),
            term: self,
        }
        .write_fmt(args)?;
        self.display();
        Ok(())
    }
}

#[doc(hidden)]
pub fn _termprint(arguments: Arguments) {
    _ = TERMINAL_EMU
        .as_ref()
        .map(|term| term.lock_no_irq(|mut term| term.write_fmt(arguments)));
}

#[macro_export]
/// Raw print to the terminal without anything appended.
macro_rules! __tprint {
    ($($arg:tt)*) => ($crate::terminal::_termprint(format_args!($($arg)*)));
}

#[macro_export]
/// Print a raw new line to the terminal.
macro_rules! __tprintln {
    () => {
        $crate::__tprint!("\n")
    };
    ($($arg:tt)*) => ($crate::__tprint!("{}\n", format_args!($($arg)*)));
}

pub use __tprint;
pub use __tprintln;

impl vte::Perform for Terminal {
    fn print(&mut self, c: char) {
        self.add_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0xa => self.add_char('\n'),
            0x0d => {
                self.col = 0;
            }
            0x09 => {
                self.add_char('\t');
            }
            byte => logging::warn!(Terminal, "[execute] {:02x}", byte),
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, c: char) {
        match c {
            'H' => {
                let mut iter = params.into_iter();
                let y = (iter.next().unwrap_or(&[1])[0].max(1) - 1) as u32;
                let x = (iter.next().unwrap_or(&[1])[0].max(1) - 1) as u32;
                self.col = x;
                self.line = y;
            }
            'A' | 'B' | 'D' | 'C' => {
                let mut params = params.into_iter();
                let amount = params.next().unwrap_or(&[1])[0] as u32;

                match c {
                    'A' => {
                        self.line -= amount;
                    }
                    'B' => self.line += amount,
                    'C' => self.col += amount,
                    'D' => self.col -= amount,
                    _ => unreachable!(),
                }
            }

            'm' => {
                let params = params.into_iter();
                let mut params_single = params.map(|p| p.get(0).copied().unwrap_or_default());
                while let Some(param) = params_single.next() {
                    match param {
                        0 => {
                            self.bg_color = RGB::BG_COLOR;
                            self.fg_color = RGB::FG_COLOR;
                        }
                        color @ 30..=37
                        | color @ 40..=47
                        | color @ 90..=97
                        | color @ 100..=107
                        | color @ 39
                        | color @ 49 => {
                            let is_bright = color >= 90;
                            let is_bg = color > 39 && (color < 90 || color >= 100);

                            let generic = if is_bg && is_bright {
                                color - 100
                            } else if is_bg {
                                color - 40
                            } else if is_bright {
                                color - 90
                            } else {
                                color - 30
                            };

                            let pix = match (generic, is_bright) {
                                (0, false) => BLACK,
                                (0, true) => BRIGHT_BLACK,
                                (1, false) => RED,
                                (1, true) => BRIGHT_RED,
                                (2, false) => GREEN,
                                (2, true) => BRIGHT_GREEN,
                                (3, false) => YELLOW,
                                (3, true) => BRIGHT_YELLOW,
                                (4, false) => BLUE,
                                (4, true) => BRIGHT_BLUE,
                                (5, false) => MAGENTA,
                                (5, true) => BRIGHT_MAGENTA,
                                (6, false) => CYAN,
                                (6, true) => BRIGHT_CYAN,
                                (7, false) => WHITE,
                                (7, true) => BRIGHT_WHITE,
                                (9, false) if !is_bg => RGB::FG_COLOR,
                                (9, false) if is_bg => RGB::BG_COLOR,
                                (g, b) => {
                                    unreachable!("color value is {g}, bright: {b}, is_bg: {is_bg}")
                                }
                            };

                            if !is_bg {
                                self.fg_color = pix;
                            } else {
                                self.bg_color = pix;
                            }
                        }

                        v @ 38 | v @ 48 => match params_single.next() {
                            Some(2) => {
                                let is_bg = v == 48;
                                let r = params_single.next().unwrap_or_default() as u8;
                                let g = params_single.next().unwrap_or_default() as u8;
                                let b = params_single.next().unwrap_or_default() as u8;

                                let pix = RGB::rgb(r, g, b);
                                if !is_bg {
                                    self.fg_color = pix;
                                } else {
                                    self.bg_color = pix;
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            c => logging::warn!(
                Terminal,
                "[csi_dispatch] params={:#?}, intermediates={:?}, ignore={:?}, char={:?}",
                params,
                intermediates,
                ignore,
                c
            ),
        }
    }
}

impl logging::LoggingSink for SpinLockIrq<Terminal> {
    fn write_record(&self, record: &logging::LogRecord) {
        self.lock_no_irq(|mut this| {
            this.write_fmt(format_args!(
                "[ {mins:04}:{secs:02}:{millis:03}:{micros:03} ] [\x1b[{}m {:<5} \x1b[0m] \x1b[90m{}:\x1b[0m {}\n",
                record.level.ansii_color(),
                record.level.display_name(),
                record.subject,
                record.arguments,
                mins = record.timestamp.minutes(),
                secs = record.timestamp.subminute_secs(),
                millis = record.timestamp.subsec_millis(),
                micros = record.timestamp.submilli_micros(),
            ))
            .expect("Failed to write a log to serial")
        })
    }

    unsafe fn panic_mode(&self) -> bool {
        unsafe { self.force_unlock() }
        true
    }
}

oninit::define_routine! {
    unsafe fn ADD_LOGGER = with TERMINAL_EMU || {
        if let Some(term) = TERMINAL_EMU.as_ref() {
            logging::register_logger(logging::LogLevel::Debug, term);
            logging::debug!(Terminal, "Registered logger");
        }
    };
}
