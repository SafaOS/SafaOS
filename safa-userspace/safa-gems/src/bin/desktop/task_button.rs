use libgem::image::{PixelImage, ScaleType};
use libgems::{
    BoundingRect, Data, EventCtx, Point, ShardEvent,
    render::{PaintBrush, shapes::Circle},
    shards::{LayoutCtx, LifeCycleCtx, RenderCtx, Shard, ShardLayout, lifecycle::LifeCycle},
    theme,
    tiny_skia::ColorU8,
};

const BTN_SIZE: u32 = 32;

const MASK: u8 = 0x2F;

pub struct TaskButton {
    icon: PixelImage,
    pub(crate) win_id: u16,
    focused: bool,
    needs_redraw: bool,
}

impl TaskButton {
    pub fn new(win_id: u16, mut icon: PixelImage) -> Self {
        icon.scale(BTN_SIZE, BTN_SIZE, ScaleType::Triangle);
        Self {
            icon,
            win_id,
            focused: false,
            needs_redraw: true,
        }
    }

    pub const fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
        self.needs_redraw = true;
    }
}

const STATUS_RADIUS: f32 = 2.5;

impl<S, M> Shard<S, M> for TaskButton {
    fn dirty(&self) -> bool {
        self.needs_redraw
    }
    fn layout(&mut self, _: &mut LayoutCtx) -> ShardLayout {
        ShardLayout {
            bounds: BoundingRect::new(
                BTN_SIZE as f32,
                (BTN_SIZE + 4) as f32 + (STATUS_RADIUS * STATUS_RADIUS),
            ),
            ..Default::default()
        }
    }

    fn render(&mut self, ctx: &mut RenderCtx, data: &Data<S, M>) -> Option<(Point, BoundingRect)> {
        let env = data.env();
        let is_hovering = ctx.is_hot() && !ctx.is_active();
        let pixmap = ctx.pixmap();

        let width = pixmap.width();

        for (y, row) in self.icon.iter_rows_from(0).enumerate() {
            let px_row = &mut pixmap.pixels_mut()[(y as u32 * width) as usize..];
            for (px, a) in px_row.iter_mut().zip(row.iter()) {
                let mask = if is_hovering { MASK } else { 0 };

                *px = ColorU8::from_rgba(
                    a.red() | mask,
                    a.green() | mask,
                    a.blue() | mask,
                    a.alpha(),
                )
                .premultiply();
            }
        }

        ctx.move_by(Point::new(width as f32 / 2., (BTN_SIZE + 4) as f32))
            .fill(
                &PaintBrush::Color(match self.focused {
                    false => env.get(theme::BACKGROUND_COLOR_1),
                    true => env.get(theme::ACCENT_COLOR_4),
                }),
                &Circle::new(STATUS_RADIUS),
            );
        self.needs_redraw = false;
        None
    }

    fn on_event(&mut self, event_ctx: &mut EventCtx, event: &ShardEvent, _: &mut Data<S, M>) {
        match event {
            ShardEvent::MouseClick(_) => {
                event_ctx.set_active(true);
                self.needs_redraw = true;
            }
            ShardEvent::MouseRelease(_) => {
                event_ctx.set_active(false);
                self.needs_redraw = true;
            }
            _ => {}
        }
    }

    fn lifecycle(&mut self, _: &mut LifeCycleCtx, event: &LifeCycle, _: &Data<S, M>) {
        match event {
            LifeCycle::HotChanged(_) => self.needs_redraw = true,
            _ => {}
        }
    }
}
