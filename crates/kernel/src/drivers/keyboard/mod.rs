pub mod keys;
pub mod set1;
pub mod usb_kbd;

use heapless::Vec;

use crate::{devices, terminal::FRAMEBUFFER_TERMINAL, utils::locks::RwLock};
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

    pub fn clear_keys(&mut self) {
        self.current_keys.clear();
    }

    #[inline]
    fn reset_unencoded_buffer(&mut self) {
        self.latest_unencoded_byte = 0;
        self.current_unencoded_key = [0; 8];
    }

    /// Adds a pressed key to the keyboard driver, returns an Err(keycode) if a key was removed, Ok(key) if a key was added
    #[must_use]
    fn add_pressed_keycode(&mut self, code: KeyCode) -> Result<Key, KeyCode> {
        // the 'lock' in capslock
        if code == KeyCode::CapsLock && self.code_is_pressed(code) {
            self.remove_pressed_keycode(code);
            return Err(code);
        }

        let key = self.process_keycode(code);
        let attempt = self.current_keys.push(key);
        if attempt.is_err() {
            *self.current_keys.last_mut().unwrap() = attempt.unwrap_err();
        }
        Ok(key)
    }

    fn remove_pressed_keycode(&mut self, code: KeyCode) {
        if code == KeyCode::Null {
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

    pub fn code_is_pressed(&self, code: KeyCode) -> bool {
        for ckey in &self.current_keys {
            if ckey.code == code {
                return true;
            }
        }
        false
    }

    #[allow(unused)]
    pub fn process_byte<T: ProcessUnencodedKeyByte>(
        &mut self,
        byte: u8,
    ) -> Option<Result<Key, KeyCode>> {
        T::process_byte(self, byte)
    }
}

pub trait HandleKey {
    fn handle_key(&mut self, key: Key);
}

// whenever a key is pressed this function should be called
// this executes a few other kernel-functions
pub fn key_pressed(key: Key) {
    if let Some(mut writer) = FRAMEBUFFER_TERMINAL.try_write() {
        writer.handle_key(key);
    };

    devices::input::keyboard::on_key_press(key.code);
}

// whenever a key is released this function should be called
// this executes a few other kernel-functions
pub fn key_release(keycode: KeyCode) {
    devices::input::keyboard::on_key_release(keycode);
}
