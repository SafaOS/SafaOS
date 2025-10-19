use super::Keyboard;
use bitflags::bitflags;

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
    pub const CTRL_KEY: Key = Self::new(KeyCode::Ctrl, KeyFlags::empty());
    pub const SHIFT_KEY: Key = Self::new(KeyCode::Shift, KeyFlags::empty());
    pub const ALT_KEY: Key = Self::new(KeyCode::Alt, KeyFlags::empty());
    pub const CAPSLOCK_KEY: Key = Self::new(KeyCode::CapsLock, KeyFlags::empty());

    pub const fn new(code: KeyCode, flags: KeyFlags) -> Self {
        Self { code, flags }
    }
}

pub use safa_abi::input::KeyCode;

pub trait EncodeKey: Sized {
    fn encode(self) -> KeyCode;
}

/// Adds a byte to the encode key buffer of the keyboard and processes it,
///
/// returns Some(Ok(key)) if a key was pressed or Some(Err(key)) if a key was released, None if nothing happened
pub trait ProcessUnencodedKeyByte: Sized {
    fn process_byte(keyboard: &mut Keyboard, byte: u8) -> Option<Result<Key, KeyCode>>;
}
