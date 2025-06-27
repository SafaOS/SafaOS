#![allow(clippy::mixed_case_hex_literals)]
use bitflags::bitflags;
use core::fmt::{Display, LowerHex, UpperHex};

use super::Keyboard;

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
    ($row: expr_2021) => {
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

/// Adds a byte to the encode key buffer of the keyboard and processes it,
///
/// returns a key, calls [`Keyboard::add_pressed_keycode`], and clears the encode key buffer if a key was pressed,
/// returns Key;:NULL if a key was unpressed or nothing happened,
///
/// calls [`Keyboard::remove_pressed_keycode`] if a key was unpressed
pub trait ProcessUnencodedKeyByte: Sized {
    fn process_byte(keyboard: &mut Keyboard, byte: u8) -> Key;
}
