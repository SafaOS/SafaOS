use safa_abi::input::{MiceBtnStatus, MiceEvent, MouseEventKind};

use crate::{devices::input, drivers::xhci::usb_hid::USBHIDDriver};

#[derive(Debug, Clone, Copy)]
pub struct USBMouseDriver;

impl USBHIDDriver for USBMouseDriver {
    fn create() -> Self
    where
        Self: Sized,
    {
        Self
    }

    fn on_event(&mut self, data: &[u8]) {
        let data = &data[..3];
        if data == &[0, 0, 0] {
            return;
        }

        let buttons = data[0];
        let x = data[1] as i8;
        let y = data[2] as i8;

        let left_is_pressed = buttons & 0b1 != 0;
        let right_is_pressed = buttons & 0b10 != 0;
        let middle_is_pressed = buttons & 0b100 != 0;

        let mut btn_status = MiceBtnStatus::NO_BUTTONS;
        if left_is_pressed {
            btn_status = btn_status.or(MiceBtnStatus::BTN_LEFT);
        }
        if right_is_pressed {
            btn_status = btn_status.or(MiceBtnStatus::BTN_RIGHT);
        }
        if middle_is_pressed {
            btn_status = btn_status.or(MiceBtnStatus::BTN_MID);
        }

        let event = MiceEvent {
            kind: MouseEventKind::Change,
            buttons_status: btn_status,
            x_rel_change: x as i16,
            // Negative means up here and we want down to be negative
            y_rel_change: (-y) as i16,
        };

        input::mouse::on_mice_change(event);
    }
}
