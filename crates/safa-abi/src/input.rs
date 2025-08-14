//! Input Devices related structures

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum KeyEventKind {
    Null = 0,
    Press,
    Release,
}

/// A Key event sent by a Keyboard Driver
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KeyEvent {
    pub kind: KeyEventKind,
    pub code: KeyCode,
}

impl KeyEvent {
    /// Constructs a null Key event
    pub const fn null() -> Self {
        Self {
            kind: KeyEventKind::Null,
            code: KeyCode::Null,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum KeyCode {
    Null = 0,
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

    Esc,
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

    KeyQ,
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

    KeyA,
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

    KeyZ,
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

    Tab,
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

    PageUp,
    PageDown,
    Insert,
    Delete,
    Home,
    End,

    // used to figure out Max of KeyCode
    LastKey,
}
