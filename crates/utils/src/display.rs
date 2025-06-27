#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RGB(u32);
impl RGB {
    #[inline(always)]
    pub const fn into_u32(self) -> u32 {
        self.0
    }

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
    #[inline(always)]
    fn from(rgb: RGB) -> Self {
        rgb.into_u32()
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
    pub const BG_COLOR: RGB = Self::BLACK;
    pub const FG_COLOR: RGB = Self::BRIGHT_WHITE;
    pub const BLACK: RGB = RGB::from_hex(0x282828);
    pub const BRIGHT_BLACK: RGB = RGB::from_hex(0x928374);

    pub const WHITE: RGB = RGB::from_hex(0xa89984);
    pub const BRIGHT_WHITE: RGB = RGB::from_hex(0xebdbb2);

    pub const RED: RGB = RGB::from_hex(0xcc241d);
    pub const BRIGHT_RED: RGB = RGB::from_hex(0xfb4934);

    pub const GREEN: RGB = RGB::from_hex(0x98971a);
    pub const BRIGHT_GREEN: RGB = RGB::from_hex(0xb8bb26);

    pub const BLUE: RGB = RGB::from_hex(0x458588);
    pub const BRIGHT_BLUE: RGB = RGB::from_hex(0x83a598);

    pub const YELLOW: RGB = RGB::from_hex(0xd79921);
    pub const BRIGHT_YELLOW: RGB = RGB::from_hex(0xfabd2f);

    pub const CYAN: RGB = RGB::from_hex(0x689d6a);
    pub const BRIGHT_CYAN: RGB = RGB::from_hex(0x8ec07c);

    pub const MAGENTA: RGB = RGB::from_hex(0xb16286);
    pub const BRIGHT_MAGENTA: RGB = RGB::from_hex(0xd3869b);
}
