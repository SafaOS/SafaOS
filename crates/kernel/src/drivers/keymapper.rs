// this driver simply converts Keys into UTF8 characters
// it is supposed to read a mapping from a file first but for now we will have a hardcoded built in
// each KeyCode is an index to 16 different MappingEntries each MappingEntry has flags and a result
// UTF8 char
// default mapping in `DEFAULT_MAPPING` const

use super::keyboard::keys::{Key, KeyCode, KeyFlags};

#[derive(Clone, Copy)]
pub struct MappingEntry {
    pub flags: KeyFlags,
    pub result: &'static str,
}

impl MappingEntry {
    pub const fn default() -> Self {
        MappingEntry {
            flags: KeyFlags::empty(),
            result: "",
        }
    }
}

#[derive(Clone, Copy)]
pub struct KeyMapping {
    pub keys: [[MappingEntry; 16]; KeyCode::LastKey as usize],
}
impl KeyMapping {
    const fn get(&mut self, index: KeyCode) -> &mut [MappingEntry] {
        &mut self.keys[index as usize]
    }

    const fn get_const(&self, index: KeyCode) -> &[MappingEntry] {
        &self.keys[index as usize]
    }
}

// beatuiful macro to create Mappings
macro_rules! create_mapping {
    ($({ $keycode: path,  $({ $($keyflag: path),* })|* } => $char: expr ),* $(,)?) => {
        {

        let mut keymapping: KeyMapping = KeyMapping {keys: [[MappingEntry::default(); 16]; KeyCode::LastKey as usize]};
        $(
            let mappings = keymapping.get($keycode);
            let mut i = 0;

            $(
                while i < mappings.len() {
                    if mappings[i].result.is_empty() {
                        break;
                    }
                    i+=1;
                }
                mappings[i] = MappingEntry {
                    flags: KeyFlags::from_bits_retain(0 $(| $keyflag.bits())*),
                    result: ($char),
                };
            )*
        )*

        keymapping
        }
    };
}

// remmber ONLY 16 mapping allowed for each keycode, each should have different flags
pub const DEFAULT_MAPPING: KeyMapping = create_mapping!(
    // Key Q mappings
    { KeyCode::KeyQ, {} } => "q",
    { KeyCode::KeyQ, { KeyFlags::CAPS_LOCK } | { KeyFlags::SHIFT } } => "Q",

    // Key W mappings
    { KeyCode::KeyW, {} } => "w",
    { KeyCode::KeyW, { KeyFlags::CAPS_LOCK } } => "W",

    // Key E mappings
    { KeyCode::KeyE, {} } => "e",
    { KeyCode::KeyE, { KeyFlags::CAPS_LOCK } } => "E",

    // Key R mappings
    { KeyCode::KeyR, {} } => "r",
    { KeyCode::KeyR, { KeyFlags::CAPS_LOCK } } => "R",

    // Key T mappings
    { KeyCode::KeyT, {} } => "t",
    { KeyCode::KeyT, { KeyFlags::CAPS_LOCK } } => "T",

    // Key Y mappings
    { KeyCode::KeyY, {} } => "y",
    { KeyCode::KeyY, { KeyFlags::CAPS_LOCK } } => "Y",

    // Key U mappings
    { KeyCode::KeyU, {} } => "u",
    { KeyCode::KeyU, { KeyFlags::CAPS_LOCK } } => "U",

    // Key I mappings
    { KeyCode::KeyI, {} } => "i",
    { KeyCode::KeyI, { KeyFlags::CAPS_LOCK } } => "I",

    // Key O mappings
    { KeyCode::KeyO, {} } => "o",
    { KeyCode::KeyO, { KeyFlags::CAPS_LOCK } } => "O",

    // Key P mappings
    { KeyCode::KeyP, {} } => "p",
    { KeyCode::KeyP, { KeyFlags::CAPS_LOCK } } => "P",

    // Key A mappings
    { KeyCode::KeyA, {} } => "a",
    { KeyCode::KeyA, { KeyFlags::CAPS_LOCK } } => "A",

    // Key S mappings
    { KeyCode::KeyS, {} } => "s",
    { KeyCode::KeyS, { KeyFlags::CAPS_LOCK } } => "S",

    // Key D mappings
    { KeyCode::KeyD, {} } => "d",
    { KeyCode::KeyD, { KeyFlags::CAPS_LOCK } } => "D",

    // Key F mappings
    { KeyCode::KeyF, {} } => "f",
    { KeyCode::KeyF, { KeyFlags::CAPS_LOCK } } => "F",

    // Key G mappings
    { KeyCode::KeyG, {} } => "g",
    { KeyCode::KeyG, { KeyFlags::CAPS_LOCK } } => "G",

    // Key H mappings
    { KeyCode::KeyH, {} } => "h",
    { KeyCode::KeyH, { KeyFlags::CAPS_LOCK } } => "H",

    // Key J mappings
    { KeyCode::KeyJ, {} } => "j",
    { KeyCode::KeyJ, { KeyFlags::CAPS_LOCK } } => "J",

    // Key K mappings
    { KeyCode::KeyK, {} } => "k",
    { KeyCode::KeyK, { KeyFlags::CAPS_LOCK } } => "K",

    // Key L mappings
    { KeyCode::KeyL, {} } => "l",
    { KeyCode::KeyL, { KeyFlags::CAPS_LOCK } } => "L",

    // Key Z mappings
    { KeyCode::KeyZ, {} } => "z",
    { KeyCode::KeyZ, { KeyFlags::CAPS_LOCK } } => "Z",

    // Key X mappings
    { KeyCode::KeyX, {} } => "x",
    { KeyCode::KeyX, { KeyFlags::CAPS_LOCK } } => "X",

    // Key C mappings
    { KeyCode::KeyC, {} } => "c",
    { KeyCode::KeyC, { KeyFlags::CAPS_LOCK } } => "C",

    // Key V mappings
    { KeyCode::KeyV, {} } => "v",
    { KeyCode::KeyV, { KeyFlags::CAPS_LOCK } } => "V",

    // Key B mappings
    { KeyCode::KeyB, {} } => "b",
    { KeyCode::KeyB, { KeyFlags::CAPS_LOCK } } => "B",

    // Key N mappings
    { KeyCode::KeyN, {} } => "n",
    { KeyCode::KeyN, { KeyFlags::CAPS_LOCK } } => "N",

    // Key M mappings
    { KeyCode::KeyM, {} } => "m",
    { KeyCode::KeyM, { KeyFlags::CAPS_LOCK } } => "M",

    // Key 1 mappings
    { KeyCode::Key1, {} } => "1",
    { KeyCode::Key1, { KeyFlags::SHIFT } } => "!",
    { KeyCode::Key1, { KeyFlags::ALT } } => "¡",

    // Key 2 mappings
    { KeyCode::Key2, {} } => "2",
    { KeyCode::Key2, { KeyFlags::SHIFT } } => "@",
    { KeyCode::Key2, { KeyFlags::ALT } } => "²",

    // Key 3 mappings
    { KeyCode::Key3, {} } => "3",
    { KeyCode::Key3, { KeyFlags::SHIFT } } => "#",
    { KeyCode::Key3, { KeyFlags::ALT } } => "³",

    // Key 4 mappings
    { KeyCode::Key4, {} } => "4",
    { KeyCode::Key4, { KeyFlags::SHIFT } } => "$",

    // Key 5 mappings
    { KeyCode::Key5, {} } => "5",
    { KeyCode::Key5, { KeyFlags::SHIFT } } => "%",

    // Key 6 mappings
    { KeyCode::Key6, {} } => "6",
    { KeyCode::Key6, { KeyFlags::SHIFT } } => "^",

    // Key 7 mappings
    { KeyCode::Key7, {} } => "7",
    { KeyCode::Key7, { KeyFlags::SHIFT } } => "&",

    // Key 8 mappings
    { KeyCode::Key8, {} } => "8",
    { KeyCode::Key8, { KeyFlags::SHIFT } } => "*",

    // Key 9 mappings
    { KeyCode::Key9, {} } => "9",
    { KeyCode::Key9, { KeyFlags::SHIFT } } => "(",

    // Key 0 mappings
    { KeyCode::Key0, {} } => "0",
    { KeyCode::Key0, { KeyFlags::SHIFT } } => ")",

    // Minus mappings
    { KeyCode::Minus, {} } => "-",
    { KeyCode::Minus, { KeyFlags::SHIFT } } => "_",

    // Equals mappings
    { KeyCode::Equals, {} } => "=",
    { KeyCode::Equals, { KeyFlags::SHIFT } } => "+",

    // Backspace mappings
    { KeyCode::Backspace, {} } => "\x08", // backspace character

    // Tab mappings
    { KeyCode::Tab, {} } => "\t", // tab character

    // Enter mappings
    { KeyCode::Return, {} } => "\n", // newline character

    // Space mappings
    { KeyCode::Space, {} } => " ",

    // Left brace mappings
    { KeyCode::LeftBrace, {} } => "[",
    { KeyCode::LeftBrace, { KeyFlags::SHIFT } } => "{",

    // Right brace mappings
    { KeyCode::RightBrace, {} } => "]",
    { KeyCode::RightBrace, { KeyFlags::SHIFT } } => "}",

    // Backslash mappings
    { KeyCode::BackSlash, {} } => "\\",
    { KeyCode::BackSlash, { KeyFlags::SHIFT } } => "|",

    // Semicolon mappings
    { KeyCode::Semicolon, {} } => ";",
    { KeyCode::Semicolon, { KeyFlags::SHIFT } } => ":",

    // Single quote mappings
    { KeyCode::DoubleQuote, {} } => "'",
    { KeyCode::DoubleQuote, { KeyFlags::SHIFT } } => "\"",

    // Back quote mappings
    { KeyCode::BackQuote, {} } => "`",
    { KeyCode::BackQuote, { KeyFlags::SHIFT } } => "~",

    // Comma mappings
    { KeyCode::Comma, {} } => ",",
    { KeyCode::Comma, { KeyFlags::SHIFT } } => "<",

    // Dot mappings
    { KeyCode::Dot, {} } => ".",
    { KeyCode::Dot, { KeyFlags::SHIFT } } => ">",

    // Slash mappings
    { KeyCode::Slash, {} } => "/",
    { KeyCode::Slash, { KeyFlags::SHIFT } } => "?",
    { KeyCode::Up, {}} => "\x1b[A",
    { KeyCode::Down, {}} => "\x1b[B",
    { KeyCode::Left, {}} => "\x1b[D",
    { KeyCode::Right, {}} => "\x1b[C",
);

impl Key {
    pub fn map_key(&self) -> &'static str {
        let mappings = DEFAULT_MAPPING.get_const(self.code);
        let mut best_mapping = None;
        let mut most_flags = KeyFlags::empty().bits();

        for mapping in mappings {
            if mapping.flags == self.flags {
                return mapping.result;
            }

            let flags_sim = (mapping.flags & self.flags).bits();

            if !mapping.result.is_empty()
                && ((mapping.flags.is_empty() && best_mapping.is_none()) || flags_sim > most_flags)
            {
                most_flags = flags_sim;
                best_mapping = Some(mapping);
            }
        }

        if let Some(mapping) = best_mapping {
            return mapping.result;
        }

        ""
    }
}
