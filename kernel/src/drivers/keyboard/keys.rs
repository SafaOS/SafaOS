#![allow(clippy::mixed_case_hex_literals)]
use core::fmt::{Display, LowerHex, UpperHex};
use int_enum::IntEnum;

use bitflags::bitflags;
use macros::EncodeKey;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(C)]
    pub struct KeyFlags: u8 {
        const CTRL = 1 << 0;
        const ALT = 1 << 1;
        const SHIFT = 1 << 2;
        const CAPS_LOCK = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode, // each code has lower 5 bits as column while the highest 3 are row
    pub flags: KeyFlags,
}

impl Key {
    pub const NULL_KEY: Key = Self::new(KeyCode::NULL, KeyFlags::empty());
    pub const CTRL_KEY: Key = Self::new(KeyCode::Ctrl, KeyFlags::empty());
    pub const SHIFT_KEY: Key = Self::new(KeyCode::Shift, KeyFlags::empty());
    pub const ALT_KEY: Key = Self::new(KeyCode::Alt, KeyFlags::empty());
    pub const CAPSLOCK_KEY: Key = Self::new(KeyCode::CapsLock, KeyFlags::empty());

    #[inline]
    pub fn is_pressed(&self) -> bool {
        super::KEYBOARD.read().is_pressed(*self)
    }

    pub const fn new(code: KeyCode, flags: KeyFlags) -> Self {
        Self { code, flags }
    }

    pub const fn default() -> Self {
        Self {
            code: KeyCode::NULL,
            flags: KeyFlags::empty(),
        }
    }
}

macro_rules! row {
    ($row: expr) => {
        $row << 5
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KeyCode {
    // set the first key at index N row to row!(N), then put the other keys in order
    NULL = row!(0),
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    PrintScr,

    Esc = row!(1),
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,
    Key0,
    Minus,
    Equals,
    Backspace,

    KeyQ = row!(2),
    KeyW,
    KeyE,
    KeyR,
    KeyT,
    KeyY,
    KeyU,
    KeyI,
    KeyO,
    KeyP,
    LeftBrace,
    RightBrace,
    BackSlash,

    KeyA = row!(3),
    KeyS,
    KeyD,
    KeyF,
    KeyG,
    KeyH,
    KeyJ,
    KeyK,
    KeyL,
    Semicolon,
    DoubleQuote,
    Return,

    KeyZ = row!(4),
    KeyX,
    KeyC,
    KeyV,
    KeyB,
    KeyN,
    KeyM,
    BackQuote,
    Comma,
    Dot,
    Slash,

    Tab = row!(5),
    CapsLock,
    Ctrl,
    Shift,
    Alt,
    Super,
    Space,
    Up,
    Down,
    Left,
    Right,

    PageUp = row!(6),
    PageDown,
    Insert,
    Delete,
    Home,
    End,

    // used to figure out Max of KeyCode
    LastKey,
}

impl KeyCode {
    #[inline]
    pub fn is_pressed(&self) -> bool {
        super::KEYBOARD.read().code_is_pressed(*self)
    }
}

impl LowerHex for KeyCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        LowerHex::fmt(&(*self as u8), f)
    }
}

impl UpperHex for KeyCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        UpperHex::fmt(&(*self as u8), f)
    }
}

impl Display for KeyCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        UpperHex::fmt(&self, f)
    }
}

pub trait EncodeKey: Sized {
    fn encode(self) -> KeyCode;
}

// you need to add the keycode as a variant below, give it the same name as the key in KeyCode enum
#[repr(u64)]
#[derive(IntEnum, Clone, Copy, EncodeKey)]
pub enum Set1Key {
    NULL = 0,

    // row 0
    F1 = 0x3B,
    F2 = 0x3C,
    F3 = 0x3D,
    F4 = 0x3E,
    F5 = 0x3F,
    F6 = 0x40,
    F7 = 0x41,
    F8 = 0x42,
    F9 = 0x43,
    F10 = 0x44,
    F11 = 0x57,
    F12 = 0x58,
    PrintScr = 0x37E02AE0,

    // row 1
    Esc = 0x1,
    Key1 = 0x2,
    Key2 = 0x3,
    Key3 = 0x4,
    Key4 = 0x5,
    Key5 = 0x6,
    Key6 = 0x7,
    Key7 = 0x8,
    Key8 = 0x9,
    Key9 = 0xA,
    Key0 = 0xB,
    Minus = 0xC,
    Equals = 0xD,
    Backspace = 0xE,

    // row 2
    KeyQ = 0x10,
    KeyW = 0x11,
    KeyE = 0x12,
    KeyR = 0x13,
    KeyT = 0x14,
    KeyY = 0x15,
    KeyU = 0x16,
    KeyI = 0x17,
    KeyO = 0x18,
    KeyP = 0x19,
    LeftBrace = 0x1A,
    RightBrace = 0x1B,
    BackSlash = 0x2B,

    // row 3
    KeyA = 0x1E,
    KeyS = 0x1F,
    KeyD = 0x20,
    KeyF = 0x21,
    KeyG = 0x22,
    KeyH = 0x23,
    KeyJ = 0x24,
    KeyK = 0x25,
    KeyL = 0x26,
    Semicolon = 0x27,
    DoubleQuote = 0x28,
    Return = 0x1C,

    // row 4
    KeyZ = 0x2C,
    KeyX = 0x2D,
    KeyC = 0x2E,
    KeyV = 0x2F,
    KeyB = 0x30,
    KeyN = 0x31,
    KeyM = 0x32,
    BackQuote = 0x29,
    Comma = 0x33,
    Dot = 0x34,
    Slash = 0x35,

    // row 5
    Tab = 0x0F,
    CapsLock = 0x3A,
    Ctrl = 0x1D,
    Shift = 0x2A,
    Alt = 0x38,
    Super = 0x5Be0,
    Space = 0x39,
    Up = 0x48e0,
    Down = 0x50e0,
    Left = 0x4Be0,
    Right = 0x4De0,

    // row 6
    PageUp = 0x49e0,
    PageDown = 0x51e0,
    Insert = 0x52e0,
    Delete = 0x53e0,
    Home = 0x47e0,
    End = 0x4Fe0,
}
