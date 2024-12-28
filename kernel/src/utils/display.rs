use macros::display_consts;

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RGB(u32);
impl RGB {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }

    pub const fn r(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub const fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    pub const fn b(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    pub const fn bytes(self) -> [u8; 3] {
        [self.r(), self.g(), self.b()]
    }

    pub const fn tuple(self) -> (u8, u8, u8) {
        (self.r(), self.g(), self.b())
    }

    #[inline(always)]
    /// returns a new color with the intensity of `intensity` the lesser the more transparent the
    /// color is into `bg`
    pub const fn with_alpha(self, intensity: u8, bg: Self) -> Self {
        let (r, g, b) = self.tuple();
        let (br, bg, bb) = bg.tuple();
        Self::new(
            ((r as u16 * intensity as u16 + br as u16 * (255 - intensity) as u16) / 255) as u8,
            ((g as u16 * intensity as u16 + bg as u16 * (255 - intensity) as u16) / 255) as u8,
            ((b as u16 * intensity as u16 + bb as u16 * (255 - intensity) as u16) / 255) as u8,
        )
    }
}

impl From<RGB> for u32 {
    fn from(rgb: RGB) -> Self {
        rgb.0
    }
}

impl From<u32> for RGB {
    fn from(u: u32) -> Self {
        RGB(u)
    }
}

impl From<[u8; 3]> for RGB {
    fn from(rgb: [u8; 3]) -> Self {
        RGB::new(rgb[0], rgb[1], rgb[2])
    }
}

impl From<RGB> for [u8; 3] {
    fn from(rgb: RGB) -> Self {
        rgb.bytes()
    }
}

impl From<(u8, u8, u8)> for RGB {
    fn from(rgb: (u8, u8, u8)) -> Self {
        RGB::new(rgb.0, rgb.1, rgb.2)
    }
}

impl From<RGB> for (u8, u8, u8) {
    fn from(rgb: RGB) -> Self {
        rgb.tuple()
    }
}

#[display_consts]
impl RGB {
    // COLORS
    pub const BLACK: RGB = RGB::new(0, 0, 0);
    pub const WHITE: RGB = RGB::new(211, 215, 207);

    pub const RED: RGB = RGB::new(204, 0, 0);
    pub const GREEN: RGB = RGB::new(78, 154, 6);
    pub const BLUE: RGB = RGB::new(114, 159, 207);

    pub const YELLOW: RGB = RGB::new(196, 160, 0);
    pub const CYAN: RGB = RGB::new(6, 152, 154);
    pub const MAGENTA: RGB = RGB::new(117, 80, 123);

    // BRIGHT COLORS
    pub const BRIGHT_BLACK: RGB = RGB::new(85, 87, 83);
    pub const BRIGHT_WHITE: RGB = RGB::new(255, 255, 255);

    pub const BRIGHT_RED: RGB = RGB::new(239, 41, 41);
    pub const BRIGHT_GREEN: RGB = RGB::new(138, 226, 52);
    pub const BRIGHT_BLUE: RGB = RGB::new(50, 175, 255);

    pub const BRIGHT_YELLOW: RGB = RGB::new(252, 233, 79);
    pub const BRIGHT_CYAN: RGB = RGB::new(52, 226, 226);
    pub const BRIGHT_MAGENTA: RGB = RGB::new(173, 127, 168);
}
