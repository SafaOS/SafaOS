#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RGB(u32);
impl RGB {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self((r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    pub const fn r(self) -> u8 {
        ((self.0 & 0xFF0000) >> 16) as u8
    }

    pub const fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    pub const fn b(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub const fn bytes(self) -> [u8; 3] {
        [self.b(), self.g(), self.r()]
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

    pub const fn from_hex(hex: u32) -> Self {
        assert!(hex <= 0xffffff);
        Self(hex)
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
        RGB::new(rgb[2], rgb[1], rgb[0])
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

impl RGB {
    pub const BLACK: RGB = RGB::from_hex(0x141a21);
    pub const BRIGHT_BLACK: RGB = RGB::from_hex(0x1d1f21);

    pub const WHITE: RGB = RGB::from_hex(0xc5c8c6);
    pub const BRIGHT_WHITE: RGB = RGB::from_hex(0xe2e6e3);

    pub const RED: RGB = RGB::from_hex(0xa02424);
    pub const BRIGHT_RED: RGB = RGB::from_hex(0xcf2f2f);

    pub const GREEN: RGB = RGB::from_hex(0x485e34);
    pub const BRIGHT_GREEN: RGB = RGB::from_hex(0x719351);

    pub const BLUE: RGB = RGB::from_hex(0x3d3dd4);
    pub const BRIGHT_BLUE: RGB = RGB::from_hex(0x3d66d4);

    pub const YELLOW: RGB = RGB::from_hex(0xc4aa37);
    pub const BRIGHT_YELLOW: RGB = RGB::from_hex(0xc4b15f);

    pub const CYAN: RGB = RGB::from_hex(0x3090a8);
    pub const BRIGHT_CYAN: RGB = RGB::from_hex(0x36a2bd);

    pub const MAGENTA: RGB = RGB::from_hex(0x973e7f);
    pub const BRIGHT_MAGENTA: RGB = RGB::from_hex(0xb54a98);
}
