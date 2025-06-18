use super::keys::{EncodeKey, KeyCode};
use crate::{
    drivers::{keyboard::KEYBOARD, xhci::usb_hid::USBHIDDriver},
    warn,
};
use int_enum::IntEnum;
use macros::EncodeKey;

// you need to add the keycode as a variant below, give it the same name as the key in KeyCode enum
// ChatGPT generated just pray it actually does work...
#[repr(u8)]
#[derive(IntEnum, Clone, Copy, EncodeKey, PartialEq, Eq)]
pub enum USBKey {
    NULL = 0x00, // Reserved

    // Function keys (F1–F12)
    F1 = 0x3A,       // 58
    F2 = 0x3B,       // 59
    F3 = 0x3C,       // 60
    F4 = 0x3D,       // 61
    F5 = 0x3E,       // 62
    F6 = 0x3F,       // 63
    F7 = 0x40,       // 64
    F8 = 0x41,       // 65
    F9 = 0x42,       // 66
    F10 = 0x43,      // 67
    F11 = 0x44,      // 68
    F12 = 0x45,      // 69
    PrintScr = 0x46, // 70

    // Row 1: Esc, 1–0, -, =, Backspace
    Esc = 0x29,       // 41
    Key1 = 0x1E,      // 30
    Key2 = 0x1F,      // 31
    Key3 = 0x20,      // 32
    Key4 = 0x21,      // 33
    Key5 = 0x22,      // 34
    Key6 = 0x23,      // 35
    Key7 = 0x24,      // 36
    Key8 = 0x25,      // 37
    Key9 = 0x26,      // 38
    Key0 = 0x27,      // 39
    Minus = 0x2D,     // 45
    Equals = 0x2E,    // 46
    Backspace = 0x2A, // 42

    // Row 2: Q–P, [ ] \
    KeyQ = 0x14,       // 20
    KeyW = 0x1A,       // 26
    KeyE = 0x08,       // 8
    KeyR = 0x15,       // 21
    KeyT = 0x17,       // 23
    KeyY = 0x1C,       // 28
    KeyU = 0x18,       // 24
    KeyI = 0x0C,       // 12
    KeyO = 0x12,       // 18
    KeyP = 0x13,       // 19
    LeftBrace = 0x2F,  // 47 (‘[’)
    RightBrace = 0x30, // 48 (‘]’)
    BackSlash = 0x31,  // 49 (‘\’)

    // Row 3: A–L, ; ' Return
    KeyA = 0x04,        // 4
    KeyS = 0x16,        // 22
    KeyD = 0x07,        // 7
    KeyF = 0x09,        // 9
    KeyG = 0x0A,        // 10
    KeyH = 0x0B,        // 11
    KeyJ = 0x0D,        // 13
    KeyK = 0x0E,        // 14
    KeyL = 0x0F,        // 15
    Semicolon = 0x33,   // 51 (‘;’)
    DoubleQuote = 0x34, // 52 (‘"’)
    Return = 0x28,      // 40

    // Row 4: Z–M, ` , . /
    KeyZ = 0x1D,      // 29
    KeyX = 0x1B,      // 27
    KeyC = 0x06,      // 6
    KeyV = 0x19,      // 25
    KeyB = 0x05,      // 5
    KeyN = 0x11,      // 17
    KeyM = 0x10,      // 16
    BackQuote = 0x35, // 53 (‘`’)
    Comma = 0x36,     // 54 (‘,’)
    Dot = 0x37,       // 55 (‘.’)
    Slash = 0x38,     // 56 (‘/’)

    // Row 5: Modifiers & navigation
    Tab = 0x2B,      // 43
    CapsLock = 0x39, // 57
    Ctrl = 0xE0,     // Left Ctrl (224)
    Shift = 0xE1,    // Left Shift (225)
    Alt = 0xE2,      // Left Alt (226)
    Super = 0xE3,    // Left GUI/Windows (227)
    Space = 0x2C,    // 44
    Up = 0x52,       // 82
    Down = 0x51,     // 81
    Left = 0x50,     // 80
    Right = 0x4F,    // 79

    // Row 6: More nav cluster
    PageUp = 0x4B,   // 75
    PageDown = 0x4E, // 78
    Insert = 0x49,   // 73
    Delete = 0x4C,   // 76
    Home = 0x4A,     // 74
    End = 0x4D,      // 77
}

#[derive(Debug)]
pub struct USBKeyboard;

impl USBHIDDriver for USBKeyboard {
    fn create() -> Self
    where
        Self: Sized,
    {
        Self
    }
    fn on_event(&self, data: &[u8]) {
        let mut report_buffer: [u8; 8] = [0; 8];
        report_buffer.copy_from_slice(&data[..8]);

        let mut keyboard = KEYBOARD.write();
        keyboard.clear_keys();

        if report_buffer == [0u8; 8] {
            return;
        }

        for byte in report_buffer {
            let usb_keycode = USBKey::try_from(byte).unwrap_or_else(|_| {
                warn!("unknown key byte with code: {:#x} encotoured", byte);
                USBKey::NULL
            });
            // also handles zero
            if usb_keycode == USBKey::NULL {
                continue;
            }

            let keycode = usb_keycode.encode();
            let key = keyboard.add_pressed_keycode(keycode);
            if let Some(key) = key {
                crate::__navi_key_pressed(key);
            }
        }
    }
}
