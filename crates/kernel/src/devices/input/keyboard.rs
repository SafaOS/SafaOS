use safa_abi::input::{KeyCode, KeyEvent, KeyEventKind};

use crate::{devices::Device, utils::locks::Mutex};

const MAX_KEY_EVENTS: usize = 4096;
pub struct KeyboardInterface {
    events: [KeyEvent; MAX_KEY_EVENTS],
    tail: usize,
    head: usize,
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

    fn next_event(&mut self) -> Option<KeyEvent> {
        if self.head == self.tail {
            return None;
        }

        let event = self.events[self.head];
        self.head += 1;
        if self.head >= MAX_KEY_EVENTS {
            self.head = 0;
        }
        Some(event)
    }
}

/// Keyboard event queue
pub static KEYBOARD_EVENT_QUEUE: Mutex<KeyboardInterface> = Mutex::new(KeyboardInterface {
    events: [KeyEvent::null(); MAX_KEY_EVENTS],
    tail: 0,
    head: 0,
});

/// Adds a key press event to the key event queue
pub fn on_key_press(keycode: KeyCode) {
    KEYBOARD_EVENT_QUEUE.lock().send_event(KeyEvent {
        kind: KeyEventKind::Press,
        code: keycode,
    });
}

/// Adds a key release event to the key event queue
pub fn on_key_release(keycode: KeyCode) {
    KEYBOARD_EVENT_QUEUE.lock().send_event(KeyEvent {
        kind: KeyEventKind::Release,
        code: keycode,
    });
}

impl Device for Mutex<KeyboardInterface> {
    fn name(&self) -> &'static str {
        "inkbd"
    }

    fn read(
        &self,
        offset: crate::drivers::vfs::SeekOffset,
        buffer: &mut [u8],
    ) -> crate::drivers::vfs::FSResult<usize> {
        _ = offset;
        let len = buffer.len();
        if !len.is_multiple_of(size_of::<KeyEvent>()) {
            // FIXME: Maybe allow that?
            return Err(crate::drivers::vfs::FSError::InvalidSize);
        }

        let count = len / size_of::<KeyEvent>();
        if count == 0 {
            return Ok(0);
        }

        let mut read_count = 0;
        let mut interface = self.lock();

        while let Some(event) = interface.next_event()
            && read_count < count
        {
            let raw_event =
                unsafe { core::mem::transmute::<_, [u8; size_of::<KeyEvent>()]>(event) };
            let read_bytes = read_count * size_of::<KeyEvent>();

            buffer[read_bytes..read_bytes + size_of::<KeyEvent>()].copy_from_slice(&raw_event);
            read_count += 1;
        }

        Ok(read_count * size_of::<KeyEvent>())
    }
}
