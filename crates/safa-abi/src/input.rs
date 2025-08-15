//! Input Devices related structures

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum MouseEventKind {
    Null = 0,
    ButtonPress = 1,
    ButtonRelease = 2,
    AxisChange = 3,
}

// TODO: should this be 32 bits? for alignment reason it will be anyways but perhaps
// I can do layout changes to all of this, I guess I need a generic layout for all kind of event producing devices?
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum MiceBtn {
    Null = 0,
    Left = 1,
    Middle = 2,
    Right = 3,
}

/// Describes a Mice change event
#[derive(Debug, Clone, Copy)]
pub struct MiceEvent {
    pub kind: MouseEventKind,
    pub button_changed: MiceBtn,
    pub x_rel_change: i16,
    pub y_rel_change: i16,
}

impl MiceEvent {
    /// Constructs a null event
    pub const fn null() -> Self {
        Self {
            kind: MouseEventKind::Null,
            button_changed: MiceBtn::Null,
            x_rel_change: 0,
            y_rel_change: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum KeyEventKind {
    Null = 0,
    Press = 1,
    Release = 2,
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
