use image::{DynamicImage, imageops};
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
    pub fn new(win_id: u16, icon: DynamicImage) -> Self {
        let icon = icon.resize_to_fill(BTN_SIZE, BTN_SIZE, imageops::FilterType::Triangle);
        let mut pixmap = tiny_skia::Pixmap::new(BTN_SIZE, BTN_SIZE)
            .expect("Failed to construct pixmap for an icon");
        let mut pixmap_hovering = pixmap.clone();

        for ((a, px), h_px) in icon
            .into_rgba8()
            .pixels()
            .zip(pixmap.pixels_mut().iter_mut())
            .zip(pixmap_hovering.pixels_mut().iter_mut())
        {
            let r = a.0[0];
            let g = a.0[1];
            let b = a.0[2];
            let a = a.0[3];

            *px = ColorU8::from_rgba(r, g, b, a).premultiply();

            // Preserves old dock behaviour by applying a mask for overlay
            // TODO: change?
            *h_px = ColorU8::from_rgba(r | MASK, g | MASK, b | MASK, a).premultiply();
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
            ctx.fill_with_pixmap(self.icon_hovering.as_ref(), &Default::default());
        } else {
            ctx.fill_with_pixmap(self.icon.as_ref(), &Default::default());
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
