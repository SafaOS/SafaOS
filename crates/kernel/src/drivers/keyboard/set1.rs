use int_enum::IntEnum;
use macros::EncodeKey;

use super::{
    keys::{EncodeKey, Key, KeyCode, KeyFlags, ProcessUnencodedKeyByte},
    Keyboard,
};

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

impl ProcessUnencodedKeyByte for Set1Key {
    fn process_byte(this: &mut Keyboard, code: u8) -> Key {
        this.current_unencoded_key[this.latest_unencoded_byte] = code;
        if code == 0xE0 {
            this.latest_unencoded_byte += 1;
            return Key::new(KeyCode::NULL, KeyFlags::empty());
        }

        let break_code = this.current_unencoded_key[this.latest_unencoded_byte] & 128 == 128;
        if break_code {
            this.current_unencoded_key[this.latest_unencoded_byte] -= 0x80;
        }

        let key: u64 = u64::from_ne_bytes(this.current_unencoded_key);
        let key = Set1Key::try_from(key).unwrap_or(Set1Key::NULL);
        let encoded = key.encode();

        this.reset_unencoded_buffer();
        if break_code {
            if encoded != KeyCode::CapsLock {
                this.remove_pressed_keycode(encoded);
            }
            Key::NULL_KEY
        } else {
            this.add_pressed_keycode(encoded).unwrap_or(Key::NULL_KEY)
        }
    }
}
