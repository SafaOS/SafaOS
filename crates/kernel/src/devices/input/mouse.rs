use safa_abi::input::MiceEvent;

use crate::{devices::Device, utils::locks::Mutex};

const MAX_MICE_EVENTS: usize = 4096;
pub struct MiceInterface {
    events: [MiceEvent; MAX_MICE_EVENTS],
    tail: usize,
    head: usize,
}
impl MiceInterface {
    fn send_event(&mut self, event: MiceEvent) {
        let place_at = self.tail;
        self.events[place_at] = event;

        self.tail += 1;
        if self.tail >= MAX_MICE_EVENTS {
            self.tail = 0;
        }
    }

    fn next_event(&mut self) -> Option<MiceEvent> {
        if self.head == self.tail {
            return None;
        }

        let event = self.events[self.head];
        self.head += 1;
        if self.head >= MAX_MICE_EVENTS {
            self.head = 0;
        }
        Some(event)
    }
}

/// Mice event queue
pub static MICE_EVENT_QUEUE: Mutex<MiceInterface> = Mutex::new(MiceInterface {
    events: [MiceEvent::null(); MAX_MICE_EVENTS],
    tail: 0,
    head: 0,
});

/// Call this function on a mouse change event
pub fn on_mice_change(event: MiceEvent) {
    MICE_EVENT_QUEUE.lock().send_event(event);
}

impl Device for Mutex<MiceInterface> {
    fn name(&self) -> &'static str {
        "inmice"
    }

    fn read(
        &self,
        offset: crate::drivers::vfs::SeekOffset,
        buffer: &mut [u8],
    ) -> crate::drivers::vfs::FSResult<usize> {
        _ = offset;
        let len = buffer.len();

        if !len.is_multiple_of(size_of::<MiceEvent>()) {
            // FIXME: Maybe allow that?
            return Err(crate::drivers::vfs::FSError::InvalidSize);
        }

        let count = len / size_of::<MiceEvent>();
        if count == 0 {
            return Ok(0);
        }

        let mut read_count = 0;
        let mut interface = self.lock();

        while let Some(event) = interface.next_event()
            && read_count < count
        {
            let raw_event =
                unsafe { core::mem::transmute::<_, [u8; size_of::<MiceEvent>()]>(event) };
            let read_bytes = read_count * size_of::<MiceEvent>();

            buffer[read_bytes..read_bytes + size_of::<MiceEvent>()].copy_from_slice(&raw_event);
            read_count += 1;
        }

        Ok(read_count * size_of::<MiceEvent>())
    }
}
