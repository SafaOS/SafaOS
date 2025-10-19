//! Input Devices related structures

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum MouseEventKind {
    Null = 0,
    /// Represents a change in the mouse status, for now the mouse doesn't report the exact event change because there could be multiple
    Change = 3, /* 3 to not collide with the keyboard's */
}

// TODO: should this be 32 bits? for alignment reason it will be anyways but perhaps
// I can do layout changes to all of this, I guess I need a generic layout for all kind of event producing devices?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct MiceBtnStatus(u32);

impl MiceBtnStatus {
    pub const BTN_LEFT: Self = Self(1);
    pub const BTN_RIGHT: Self = Self(2);
    pub const BTN_MID: Self = Self(3);
    pub const NO_BUTTONS: Self = Self(0);

    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn or(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn and(&self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn not(&self) -> Self {
        Self(!self.0)
    }
}

/// Describes a Mice change event
#[derive(Debug, Clone, Copy)]
pub struct MiceEvent {
    pub kind: MouseEventKind,
    /// The buttons status
    pub buttons_status: MiceBtnStatus,
    /// The X relative change, positive means right, negative means left
    pub x_rel_change: i16,
    /// The Y relative change, positive means up, negative means down,
    /// assuming the coordinate system has the bigger Y the more up,
    /// which isn't true for most computer software so you have to invert the Y axis.
    pub y_rel_change: i16,
}

impl MiceEvent {
    /// Constructs a null event
    pub const fn null() -> Self {
        Self {
            kind: MouseEventKind::Null,
            buttons_status: MiceBtnStatus(0),
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
