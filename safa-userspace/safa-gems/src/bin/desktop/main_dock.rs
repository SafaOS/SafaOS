use libgem::{
    Gem, LIGHT_BG_COLOR0,
    canvas::{DrawingCanvas, Pixel},
    element::Element,
    image::{BMPImage, PixelImage},
};

use crate::task_button::TaskButton;

const MIN_ITEM_HEIGHT: u32 = 48;
const MIN_ITEM_WIDTH: u32 = 32;

const PADDING_X: u32 = 8;
const PADDING_Y: u32 = 2;
const RADIUS: u32 = 8;
const DOCK_COLOR: Pixel = LIGHT_BG_COLOR0.with_alpha(0xF0);

static FALLBACK_ICON_BMP: &[u8] = include_bytes!("../../../assets/unknown.bmp");

fn fallback_icon() -> PixelImage {
    BMPImage::from_slice(FALLBACK_ICON_BMP)
        .expect("Failed to parse fallback image")
        .into()
}

pub struct MainDock {
    tasks: Vec<TaskButton>,
    /// Last draw info, used to erase the previous drawings because this isn't really static.
    last_draw: Option<((u32, u32), (u32, u32))>,
    elements_changed: bool,
}

impl MainDock {
    fn width(&self) -> u32 {
        self.tasks
            .iter()
            .map(|t| t.width() + PADDING_X)
            .sum::<u32>()
            .max(PADDING_X + MIN_ITEM_WIDTH)
            + PADDING_X
    }

    fn height(&self) -> u32 {
        MIN_ITEM_HEIGHT
    }

    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            last_draw: None,
            elements_changed: true,
        }
    }

    pub fn attached(&mut self, win: u16) {
        let Ok(info) = libopal::window::window_info(win) else {
            return;
        };

        let icon = (|| {
            if let Some(icon) = info.icon_id() {
                let Ok(raw_data) = libopal::icon::get_icon_data_bmp(icon) else {
                    return fallback_icon();
                };
                let bmp = match BMPImage::from_slice(&raw_data) {
                    Ok(k) => k,
                    Err(e) => {
                        println!("Error prasing BMP Icon: {e} for window: {}", info.name());
                        return fallback_icon();
                    }
                };

                bmp.into()
            } else {
                fallback_icon()
            }
        })();

        self.tasks.push(TaskButton::new(win, icon));
        self.elements_changed = true;
    }

    pub fn deatached(&mut self, win: u16) {
        for (i, t) in self.tasks.iter().enumerate() {
            if t.win() == win {
                self.tasks.remove(i);
                self.elements_changed = true;
                return;
            }
        }
    }

    pub fn focus_changed(&mut self, win: u16, focused: bool) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.win() == win) {
            task.set_focused(focused);
        }
    }
}

impl<Canvas: DrawingCanvas, G: Gem> Element<Canvas, G> for MainDock {
    fn draw_height(&self) -> u32 {
        self.height()
    }

    fn draw_width(&self) -> u32 {
        self.width()
    }

    fn container_height(&self) -> u32 {
        self.height()
    }

    fn container_width(&self) -> u32 {
        self.width()
    }

    fn needs_redraw(&self) -> bool {
        self.elements_changed || self.tasks.iter().any(|t| t.needs_redraw())
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

        let mut draw_started_at = None;
        let mut set_draw_start = |x: u32, y: u32| match draw_started_at {
            Some((o_x, o_y)) => draw_started_at = Some((x.min(o_x), y.min(o_y))),
            None => draw_started_at = Some((x, y)),
        };

        let mut draw_ended_at = None;
        let mut set_draw_end = |x: u32, y: u32| match draw_ended_at {
            Some((o_x, o_y)) => draw_ended_at = Some((x.max(o_x), y.max(o_y))),
            None => draw_ended_at = Some((x, y)),
        };

        if self.elements_changed {
            set_draw_start(start_x, start_y);
            set_draw_end(start_x + width, start_y + height);

            if let Some(((l_start_x, l_start_y), (l_width, l_height))) = self
                .last_draw
                .replace(((start_x, start_y), (width, height)))
                && (l_width != width
                    || l_height != height
                    || l_start_x != start_x
                    || l_start_y != start_y)
            {
                set_draw_start(l_start_x, l_start_y);
                set_draw_end(l_start_x + l_width, l_start_y + l_height);

                canvas.draw_rect(
                    l_start_x,
                    l_start_y,
                    l_width,
                    l_height,
                    Pixel::NONE,
                    Some(bg_color),
                );
            }

            canvas.draw_round_rect(
                start_x,
                start_y,
                width,
                height,
                RADIUS,
                |_, _| DOCK_COLOR,
                Some(bg_color),
            );

            self.elements_changed = false;
        }

        let mut draw_x = start_x + PADDING_X;
        let draw_y = start_y + PADDING_Y;

        for btn in self.tasks.iter_mut() {
            let (start, end) =
                <TaskButton as Element<Canvas, G>>::draw(btn, canvas, draw_x, draw_y, DOCK_COLOR);
            if let Some((x, y)) = start {
                set_draw_start(x, y);
            }

            if let Some((x, y)) = end {
                set_draw_end(x, y);
            }

            draw_x += btn.width().max(MIN_ITEM_WIDTH) + PADDING_X;
        }

        (draw_started_at, draw_ended_at)
    }

    fn handle_event(&mut self, gem: &mut G, event: libopal::Event, ele_x: u32, ele_y: u32) {
        let mut draw_x = ele_x + PADDING_X;
        let draw_y = ele_y + PADDING_Y;
        for btn in self.tasks.iter_mut() {
            <TaskButton as Element<Canvas, G>>::handle_event(btn, gem, event, draw_x, draw_y);

            draw_x += btn.width() + PADDING_X;
        }
    }
}
