pub mod keys;
pub mod set1;

use heapless::Vec;

use crate::utils::locks::RwLock;
use keys::{Key, KeyCode, KeyFlags, ProcessUnencodedKeyByte};

const MAX_KEYS: usize = 256;

pub struct Keyboard {
    current_keys: Vec<Key, MAX_KEYS>,
    latest_unencoded_byte: usize,
    current_unencoded_key: [u8; 8],
}

pub static KEYBOARD: RwLock<Keyboard> = RwLock::new(Keyboard::new());

impl Keyboard {
    pub const fn new() -> Self {
        Self {
            current_keys: Vec::new(),
            latest_unencoded_byte: 0,
            current_unencoded_key: [0; 8],
        }
    }

    #[inline]
    fn reset_unencoded_buffer(&mut self) {
        self.latest_unencoded_byte = 0;
        self.current_unencoded_key = [0; 8];
    }

    fn add_pressed_keycode(&mut self, code: KeyCode) {
        if code == KeyCode::NULL {
            return;
        }

        // the 'lock' in capslock
        if code == KeyCode::CapsLock && self.code_is_pressed(code) {
            self.remove_pressed_keycode(code);
            return;
        }

        let key = self.process_keycode(code);
        let attempt = self.current_keys.push(key);
        if attempt.is_err() {
            *self.current_keys.last_mut().unwrap() = attempt.unwrap_err();
        }
    }

    fn remove_pressed_keycode(&mut self, code: KeyCode) {
        if code == KeyCode::NULL {
            return;
        }

        let key = self
            .current_keys
            .iter()
            .enumerate()
            .find(|(_, key)| key.code == code);

        if let Some((index, _)) = key {
            self.current_keys.remove(index);
        }
    }

    // returns a Key with flags from keycode
    pub fn process_keycode(&self, keycode: KeyCode) -> Key {
        let mut flags = KeyFlags::empty();

        if self.code_is_pressed(Key::SHIFT_KEY.code) && keycode != KeyCode::Ctrl {
            flags |= KeyFlags::SHIFT;
        }

        if self.code_is_pressed(Key::CTRL_KEY.code) && keycode != KeyCode::Shift {
            flags |= KeyFlags::CTRL;
        }

        if self.code_is_pressed(Key::ALT_KEY.code) && keycode != KeyCode::Alt {
            flags |= KeyFlags::ALT;
        }

        if self.code_is_pressed(Key::CAPSLOCK_KEY.code) && keycode != KeyCode::CapsLock {
            flags |= KeyFlags::CAPS_LOCK;
        }

        Key::new(keycode, flags)
    }

    pub fn is_pressed(&self, key: Key) -> bool {
        for ckey in &self.current_keys {
            if ckey.code == key.code && ckey.flags == key.flags {
                return true;
            }
        }
        false
    }

    pub fn code_is_pressed(&self, code: KeyCode) -> bool {
        for ckey in &self.current_keys {
            if ckey.code == code {
                return true;
            }
        }
        false
    }

    pub fn process_byte<T: ProcessUnencodedKeyByte>(&mut self, byte: u8) -> Key {
        T::process_byte(self, byte)
    }
}

pub trait HandleKey {
    fn handle_key(&mut self, key: Key);
}
