use super::{
    keys::{EncodeKey, Key, KeyCode, KeyFlags, Set1Key},
    Keyboard,
};

impl Keyboard {
    pub fn handle_ps2_set_1(&mut self, code: u8) -> Key {
        self.current_unencoded_key[self.latest_unencoded_byte] = code;
        if code == 0xE0 {
            self.latest_unencoded_byte += 1;
            return Key::new(KeyCode::NULL, KeyFlags::empty());
        }

        let break_code = self.current_unencoded_key[self.latest_unencoded_byte] & 128 == 128;
        if break_code {
            self.current_unencoded_key[self.latest_unencoded_byte] -= 0x80;
        }

        let key: u64 = unsafe { core::mem::transmute(self.current_unencoded_key) };
        let key = Set1Key::try_from(key).unwrap_or(Set1Key::NULL);
        let encoded = key.encode();

        self.reset_unencoded_buffer();
        if break_code {
            if encoded != KeyCode::CapsLock {
                self.remove_pressed_keycode(encoded);
            }
            Key::NULL_KEY
        } else {
            self.add_pressed_keycode(encoded);
            self.process_keycode(encoded)
        }
    }
}
