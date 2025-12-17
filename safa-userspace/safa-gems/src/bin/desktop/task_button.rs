use libgem::{
    Gem,
    canvas::{DrawingCanvas, Pixel},
    element::{Element, is_inside_rect},
    image::{PixelImage, ScaleType, display::ARGB},
};
use libopal::event::HeldMouseButtons;

const BTN_SIZE: u32 = 32;
const BTN_HEIGHT: u32 = 48;

const FOCUSED_COLOR: Pixel = Pixel::rgb(0xb1, 0x62, 0x86);
const IDLE_COLOR: Pixel = Pixel::rgb(0x7c, 0x6f, 0x64);
const MASK: u8 = 0x2F;

pub struct TaskButton {
    icon: PixelImage,
    win_id: u16,
    status_color: Pixel,
    needs_redraw: bool,
    is_hovering: bool,
    is_held: bool,
}

impl TaskButton {
    pub fn new(win_id: u16, mut icon: PixelImage) -> Self {
        icon.scale(BTN_SIZE, BTN_SIZE, ScaleType::Triangle);
        Self {
            is_hovering: false,
            is_held: false,
            icon,
            win_id,
            status_color: IDLE_COLOR,
            needs_redraw: true,
        }
    }

    pub const fn width(&self) -> u32 {
        BTN_SIZE
    }

    pub const fn height(&self) -> u32 {
        BTN_HEIGHT
    }

    pub const fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub const fn set_focused(&mut self, focus: bool) {
        if focus {
            self.status_color = FOCUSED_COLOR;
        } else {
            self.status_color = IDLE_COLOR;
        }
        self.needs_redraw = true;
    }

    pub const fn win(&self) -> u16 {
        self.win_id
    }
}

impl<G: Gem, Canvas: DrawingCanvas> Element<Canvas, G> for TaskButton {
    fn container_width(&self) -> u32 {
        BTN_SIZE
    }

    fn container_height(&self) -> u32 {
        BTN_SIZE
    }

    fn draw_height(&self) -> u32 {
        BTN_SIZE
    }

    fn draw_width(&self) -> u32 {
        BTN_SIZE
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    fn draw(
        &mut self,
        canvas: &mut Canvas,
        start_x: u32,
        start_y: u32,
        bg_color: Pixel,
    ) -> (Option<(u32, u32)>, Option<(u32, u32)>) {
        let width = self.width();
        let height = self.height();

        canvas.draw_rect(start_x, start_y, width, height, Pixel::NONE, Some(bg_color));

        for (y, row) in self.icon.iter_rows_from(0).enumerate() {
            canvas.draw_row_iter(
                start_x,
                start_y + y as u32,
                row.iter().map(|a| {
                    if self.is_hovering {
                        ARGB::from_rgba(
                            a.red() | MASK,
                            a.green() | MASK,
                            a.blue() | MASK,
                            a.alpha(),
                        )
                    } else {
                        *a
                    }
                }),
                Some(bg_color),
            );
        }

        canvas.draw_circle_filled(
            start_x + (width / 2),
            start_y + BTN_SIZE + 4,
            2,
            self.status_color,
            Some(bg_color),
        );

        self.needs_redraw = false;
        (
            Some((start_x, start_y)),
            Some((start_x + width, start_y + height)),
        )
    }

    fn handle_event(&mut self, gem: &mut G, event: libopal::Event, ele_x: u32, ele_y: u32) {
        _ = gem;
        let width = self.width();
        let height = self.height();

        let (pos, leave, btn_left) = match event {
            libopal::Event::MouseChange(m) => (
                Some((m.x(), m.y())),
                false,
                m.buttons_changed()
                    .then_some(m.held_buttons().contains(HeldMouseButtons::LEFT)),
            ),
            libopal::Event::MouseLeave(_) => (None, true, None),
            libopal::Event::MouseEnter(m) => (Some((m.x(), m.y())), false, None),
            _ => (None, false, None),
        };

        let mut is_hovering = self.is_hovering;
        let mut is_held = self.is_held;

        if let Some((x, y)) = pos {
            is_hovering = is_inside_rect(x, y, ele_x, ele_y, width, height);
        }

        if leave {
            is_hovering = false;
        }

        if let Some(btn_left) = btn_left
            && is_hovering
        {
            if btn_left {
                is_hovering = false;
                is_held = true;
            } else if self.is_held {
                _ = libopal::window::focus_window(self.win_id);
            }
        }

        self.needs_redraw = is_hovering != self.is_hovering;

        self.is_hovering = is_hovering;
        self.is_held = is_held;
    }
}
