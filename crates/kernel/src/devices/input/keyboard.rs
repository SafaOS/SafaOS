use alloc::boxed::Box;
use safa_abi::input::{KeyCode, KeyEvent, KeyEventKind};

use crate::{
    devices::{Device, DeviceInterface},
    utils::locks::{Mutex, RwLock},
};

const MAX_KEY_EVENTS: usize = 4096;
pub struct KeyboardInterface {
    events: [KeyEvent; MAX_KEY_EVENTS],
    tail: usize,
}
impl KeyboardInterface {
    fn send_event(&mut self, event: KeyEvent) {
        let place_at = self.tail;
        self.events[place_at] = event;

        self.tail += 1;
        if self.tail >= MAX_KEY_EVENTS {
            self.tail = 0;
        }
    }

    fn event_buffer_u8(&self) -> &[u8; MAX_KEY_EVENTS * size_of::<KeyEvent>()] {
        unsafe { core::mem::transmute(&self.events) }
    }

    fn bytes_end(&self) -> usize {
        self.tail * size_of::<KeyEvent>()
    }
}

/// Keyboard event queue
pub static KEYBOARD_EVENT_QUEUE: RwLock<KeyboardInterface> = RwLock::new(KeyboardInterface {
    events: [KeyEvent::null(); MAX_KEY_EVENTS],
    tail: 0,
});

/// Adds a key press event to the key event queue
pub fn on_key_press(keycode: KeyCode) {
    KEYBOARD_EVENT_QUEUE.write().send_event(KeyEvent {
        kind: KeyEventKind::Press,
        code: keycode,
    });
}

/// Adds a key release event to the key event queue
pub fn on_key_release(keycode: KeyCode) {
    KEYBOARD_EVENT_QUEUE.write().send_event(KeyEvent {
        kind: KeyEventKind::Release,
        code: keycode,
    });
}

pub struct KeyboardPoller {
    curr_position: usize,
}

impl Device for Mutex<KeyboardPoller> {
    fn name(&self) -> &'static str {
        "kbd"
    }

    fn read(
        &self,
        offset: crate::drivers::vfs::SeekOffset,
        buffer: &mut [u8],
    ) -> crate::drivers::vfs::FSResult<usize> {
        _ = offset;
        let mut this = self.lock();
        let interface = KEYBOARD_EVENT_QUEUE.read();

        let raw_keyevents = interface.event_buffer_u8();
        let tail_off = interface.bytes_end();
        if tail_off == this.curr_position {
            return Ok(0);
        }

        let len = buffer.len().min(raw_keyevents.len() - this.curr_position);
        let len = if tail_off > this.curr_position {
            len.min(tail_off - this.curr_position)
        } else {
            len
        };

        buffer[..len].copy_from_slice(&raw_keyevents[this.curr_position..this.curr_position + len]);

        this.curr_position += len;
        if this.curr_position >= raw_keyevents.len() {
            this.curr_position = 0;
        }

        Ok(len)
    }
}

impl DeviceInterface for RwLock<KeyboardInterface> {
    fn name(&self) -> &'static str {
        "inkbd"
    }
    fn open(&self) -> alloc::boxed::Box<dyn Device> {
        let read = self.read();
        Box::new(Mutex::new(KeyboardPoller {
            curr_position: read.bytes_end(),
        }))
    }
}
