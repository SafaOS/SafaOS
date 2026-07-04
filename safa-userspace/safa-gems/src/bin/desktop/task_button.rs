use libgem::image::{PixelImage, ScaleType};
use libgems::{
    BoundingRect, Color, Data, EventCtx, Point, ShardEvent,
    render::shapes::Circle,
    shards::{LayoutCtx, LifeCycleCtx, RenderCtx, Shard, ShardLayout, lifecycle::LifeCycle},
    tiny_skia::{self, ColorU8},
};

const BTN_SIZE: u32 = 32;

const MASK: u8 = 0x2F;

const IDLE_COLOR: Color = Color::rgb(0x7c, 0x6f, 0x64);
const FOCUSED_COLOR: Color = Color::rgb(0xb1, 0x62, 0x86);
pub struct TaskButton {
    icon: tiny_skia::Pixmap,
    icon_hovering: tiny_skia::Pixmap,
    pub(crate) win_id: u16,
    focused: bool,
    needs_redraw: bool,
}

impl TaskButton {
    pub fn new(win_id: u16, mut icon: PixelImage) -> Self {
        icon.scale(BTN_SIZE, BTN_SIZE, ScaleType::Triangle);
        let mut pixmap = tiny_skia::Pixmap::new(BTN_SIZE, BTN_SIZE)
            .expect("Failed to construct pixmap for an icon");
        let mut pixmap_hovering = pixmap.clone();
        let width = pixmap.width();

        for (y, row) in icon.iter_rows_from(0).enumerate() {
            let px_row = &mut pixmap.pixels_mut()[(y as u32 * width) as usize..];
            let masked_px_row = &mut pixmap_hovering.pixels_mut()[(y as u32 * width) as usize..];
            for ((px, h_px), a) in px_row
                .iter_mut()
                .zip(masked_px_row.iter_mut())
                .zip(row.iter())
            {
                *px = ColorU8::from_rgba(a.red(), a.green(), a.blue(), a.alpha()).premultiply();

                // Preserves old dock behaviour by applying a mask for overlay
                // TODO: change?
                *h_px = ColorU8::from_rgba(
                    a.red() | MASK,
                    a.green() | MASK,
                    a.blue() | MASK,
                    a.alpha(),
                )
                .premultiply();
            }
        }
        Self {
            icon: pixmap,
            icon_hovering: pixmap_hovering,
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

    fn render(&mut self, ctx: &mut RenderCtx, _: &Data<S, M>) {
        let is_hovering = ctx.is_hot() && !ctx.is_active();
        let width = ctx.layout().bounds.width();

        if is_hovering {
            ctx.fill_with_pixmap(self.icon_hovering.as_ref());
        } else {
            ctx.fill_with_pixmap(self.icon.as_ref());
        }

        ctx.move_to(Point::new(width / 2., (BTN_SIZE + 4) as f32))
            .fill(
                &match self.focused {
                    false => IDLE_COLOR,
                    true => FOCUSED_COLOR,
                }
                .into(),
                &Circle::new(STATUS_RADIUS),
            );
        self.needs_redraw = false;
    }

    fn on_event(&mut self, event_ctx: &mut EventCtx, event: &ShardEvent, _: &mut Data<S, M>) {
        match event {
            ShardEvent::MouseClick(_) => {
                event_ctx.set_active(true);
                event_ctx.request_redraw();
            }
            ShardEvent::MouseRelease(_) => {
                event_ctx.set_active(false);
                event_ctx.request_redraw();
            }
            _ => {}
        }
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, _: &Data<S, M>) {
        match event {
            LifeCycle::HotChanged(_) => ctx.request_redraw(),
            _ => {}
        }
    }
}
