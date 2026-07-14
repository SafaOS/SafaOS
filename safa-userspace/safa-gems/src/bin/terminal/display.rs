use std::collections::VecDeque;

use crate::{FONT_HEIGHT, FONT_WIDTH, LINE_HEIGHT, play_beep, utils};

use libgems::{
    BoundingRect, Color, Data, Point, cosmic_text,
    render::PaintBrush,
    shards::{LayoutCtx, MsgCtx, Shard, ShardLayout, UpdateCtx},
    tiny_skia::{self, Rect},
};

/// Extra attributes for a glyph
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExtraAttributes {
    extra_flags: u32,
    background_color: Pixel,
}

impl ExtraAttributes {
    pub const fn from_usize(u: usize) -> Self {
        unsafe { core::mem::transmute(u) }
    }

    pub const fn raw(self) -> usize {
        unsafe { core::mem::transmute(self) }
    }

    /// Whether or not we can skip rendering this glyph
    pub const fn should_skip(&self) -> bool {
        (self.extra_flags & 1) == 1
    }

    pub const fn with_should_skip(mut self, t: bool) -> Self {
        if t {
            self.extra_flags &= 1;
        } else {
            self.extra_flags &= !1;
        }
        self
    }

    /// Returns the background color of the cell containing the glyph or None if the background is transparent
    pub const fn bg(&self) -> Option<Pixel> {
        if unsafe { core::mem::transmute::<Pixel, u32>(self.background_color) } != 0 {
            Some(self.background_color)
        } else {
            None
        }
    }

    pub const fn with_bg(mut self, bg: Pixel) -> Self {
        self.background_color = bg;
        self
    }
}

/// Converts a pixel to a cosmic_text::Color
/// Pixel is premultiplied-alpha while a color is the opposite
const fn pix_to_color(pix: Color) -> cosmic_text::Color {
    cosmic_text::Color::rgba(pix.r(), pix.g(), pix.b(), pix.a())
}

/// Converts a cosmic_text::color to a Pixel
/// Pixel is premultiplied-alpha while a color is the opposite
const fn color_to_pix(color: cosmic_text::Color) -> Pixel {
    Pixel::hex_rgba(color.0)
}

fn default_attrs() -> cosmic_text::Attrs<'static> {
    cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cell {
    c: char,
    bg: Color,
    fg: Color,
    weight: cosmic_text::Weight,
    _attr: u16,
}

impl Cell {
    pub fn should_skip(&self) -> bool {
        (self._attr & 1) == 1
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            bg: Color::NONE,
            fg: Color::NONE,
            weight: cosmic_text::Weight::default(),
            _attr: 1,
        }
    }
}

#[derive(Debug, Clone)]
struct Row {
    cells: Box<[Cell]>,
    text: String,
    buffer: cosmic_text::Buffer,
    dirty: bool,
    empty: bool,
}

impl Row {
    fn empty(len: u32) -> Self {
        Row {
            cells: vec![Cell::default(); len as usize].into_boxed_slice(),
            buffer: cosmic_text::Buffer::new_empty(cosmic_text::Metrics::new(
                FONT_HEIGHT as f32,
                LINE_HEIGHT as f32,
            )),
            text: String::with_capacity(len as usize),
            dirty: false,
            empty: true,
        }
    }
    pub fn prepare_data(
        &mut self,
        font_sys: &mut cosmic_text::FontSystem,
    ) -> &mut cosmic_text::Buffer {
        if self.dirty {
            if !self.empty {
                let mut attrs_list = cosmic_text::AttrsList::new(&default_attrs());

                let string = &mut self.text;
                string.clear();

                let mut last_cell = None;
                for (i, cell) in self.cells.iter().enumerate() {
                    string.push(cell.c);

                    if last_cell.is_none_or(|c: &Cell| {
                        c._attr != cell._attr
                            || c.bg != cell.bg
                            || c.fg != cell.fg
                            || c.weight != cell.weight
                    }) {
                        attrs_list.add_span(
                            i..self.cells.len(),
                            &default_attrs()
                                .weight(cell.weight)
                                .color(pix_to_color(cell.fg))
                                .metadata(
                                    ExtraAttributes::from_usize(0)
                                        .with_bg(cell.bg)
                                        .with_should_skip(cell.should_skip())
                                        .raw(),
                                ),
                        );
                    }
                    last_cell = Some(cell);
                }

                self.buffer
                    .set_monospace_width(font_sys, Some(FONT_WIDTH as f32));
                self.buffer.set_rich_text(
                    font_sys,
                    attrs_list
                        .spans_iter()
                        .map(|(span, attrs)| (&string[span.start..span.end], attrs.as_attrs())),
                    &default_attrs(),
                    cosmic_text::Shaping::Advanced,
                    None,
                );

                self.buffer.set_redraw(true);
            } else {
                self.buffer
                    .set_monospace_width(font_sys, Some(FONT_WIDTH as f32));
                self.buffer.shape_until_scroll(font_sys, false);
                self.buffer.set_redraw(true);
            }

            self.dirty = false;
        }

        &mut self.buffer
    }

    pub fn clear(&mut self, start: usize, end: usize) {
        self.cells[start..end].fill(Cell::default());

        self.dirty = true;
        if start == 0 && end == self.cells.len() {
            self.buffer.lines.get_mut(0).map(|line| {
                line.set_text(
                    String::new(),
                    cosmic_text::LineEnding::None,
                    cosmic_text::AttrsList::new(&default_attrs()),
                )
            });

            self.empty = true;
        }
    }

    pub fn put(
        &mut self,
        at: usize,
        c: char,
        fg: Color,
        bg: Color,
        weight: cosmic_text::Weight,
        mut attr: u16,
    ) {
        if c == ' ' {
            attr |= 1;
        }
        let new_cell = Cell {
            c,
            fg,
            bg,
            weight,
            _attr: attr,
        };

        if core::mem::replace(&mut self.cells[at], new_cell) != new_cell {
            self.dirty = true;
        }

        if new_cell != Cell::default() {
            self.empty = false;
        }
    }
}

struct Grid {
    rows: VecDeque<Row>,
    width: u32,
}

impl Grid {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            rows: vec![Row::empty(width); height as usize].into(),
            width,
        }
    }
    pub fn shift_up(&mut self, pixmap: &mut tiny_skia::Pixmap, line_height: u32) {
        self.rows.pop_front();
        self.rows.push_back(Row::empty(self.width()));

        let width = pixmap.width() as usize;
        let stride = width * line_height as usize;
        let total = pixmap.pixels().len();
        let pixels = pixmap.pixels_mut();
        pixels.copy_within(stride.., 0); // shift everything up one row's worth
        pixels[total - stride..].fill(tiny_skia::PremultipliedColorU8::TRANSPARENT); // clear the newly exposed last row
    }
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.rows.len() as u32
    }
}

pub enum TermRequest {
    CursorRequest((u32, u32)),
}

#[derive(Debug, Clone, Copy)]
struct DrawCursor {
    x: u32,
    y: u32,
    curr_fg: Color,
    curr_bg: Color,
    curr_weight: cosmic_text::Weight,
    curr_attr: u16,
}
pub struct TermDisplay {
    pixmap: tiny_skia::Pixmap,
    grid: Grid,
    requests: Vec<TermRequest>,
    damage_min: Option<u32>,
    damage_max: Option<u32>,
    viewport_index: u32,
    viewport_lines: u32,
    cursor: DrawCursor,
    saved_cursor: DrawCursor,
    default_fg: Color,
}

impl TermDisplay {
    pub fn new(width: u32, height: u32) -> Self {
        let draw = DrawCursor {
            x: 0,
            y: 0,

            curr_fg: Color::WHITE,
            curr_bg: Color::NONE,
            curr_weight: cosmic_text::Weight::default(),
            curr_attr: 0,
        };
        Self {
            saved_cursor: draw,
            pixmap: tiny_skia::Pixmap::new(
                width * FONT_WIDTH.ceil() as u32,
                height * LINE_HEIGHT.ceil() as u32,
            )
            .expect("Failed to construct terminal bitmap"),
            grid: Grid::new(width, height),
            damage_min: None,
            damage_max: None,
            viewport_index: 0,
            viewport_lines: height,
            cursor: draw,
            default_fg: Color::WHITE,
            requests: Vec::new(),
        }
    }

    pub fn cursor(&self) -> (u32, u32) {
        (self.cursor.x, self.cursor.y)
    }

    pub fn collect_requests<'s>(&'s mut self) -> std::vec::Drain<'s, TermRequest> {
        self.requests.drain(..)
    }

    fn reset(&mut self) {
        self.cursor.curr_fg = self.default_fg;
        self.cursor.curr_bg = Color::NONE;
        self.cursor.curr_weight = cosmic_text::Weight::default();
        self.cursor.curr_attr = 0;
    }

    fn clear(&mut self, x: u32, end_x: u32, y_range: std::ops::Range<u32>, scrollback: bool) {
        for y in y_range.clone() {
            if y as usize >= self.grid.rows.len() {
                break;
            }

            let row = &mut self.grid.rows[y as usize];

            let mut start = 0;
            let mut end = row.cells.len();

            if y == y_range.start {
                start = x as usize;
            }

            if y == y_range.end {
                end = end_x as usize;
            }

            row.clear(start, end);
        }
        if scrollback {
            self.move_cursor(0, 0);
        }
    }
    fn insert_char(&mut self, c: char) {
        let (x, y) = self.cursor();
        if c == '\n' {
            return self.move_cursor(0, y + 1);
        }

        if c == '\x08' {
            match (x.checked_sub(1), y.checked_sub(1)) {
                (None, None) => return,
                (Some(x), _) => {
                    return self.move_cursor(x, y);
                }
                (None, Some(y)) => return self.move_cursor(self.grid.width() - 1, y),
            }
        }

        self.grid.rows[y as usize].put(
            x as usize,
            c,
            self.cursor.curr_fg,
            self.cursor.curr_bg,
            self.cursor.curr_weight,
            self.cursor.curr_attr,
        );

        self.move_cursor(x + 1, y);
    }

    fn collect_damage(&mut self) -> Option<(Point, BoundingRect)> {
        self.damage_min
            .take()
            .zip(self.damage_max.take())
            .map(|(min, max)| {
                (
                    Point::new(0., (min - self.viewport_index) as f32 * LINE_HEIGHT as f32),
                    BoundingRect::new(
                        (self.grid.width() as f32 * FONT_WIDTH.ceil()) as f32,
                        (max - min + 1) as f32 * LINE_HEIGHT as f32,
                    ),
                )
            })
    }

    pub fn move_view_by(&mut self, lines: i32) {
        self.viewport_index = self
            .viewport_index
            .saturating_add_signed(lines)
            .min(self.grid.height() - 1);

        self.damage_min = Some(self.viewport_index);
        self.damage_max = Some(self.viewport_lines + self.viewport_index);
    }

    fn refresh_damage(&mut self, viewport_changed: bool) {
        if viewport_changed {
            self.damage_min = Some(self.viewport_index);
            self.damage_max = Some(self.viewport_lines + self.viewport_index);
            return;
        }

        let (_, y) = self.cursor();

        if let Some(ref mut d_y) = self.damage_min {
            *d_y = (*d_y).min(y);
        } else {
            self.damage_min = Some(y);
        }

        if let Some(ref mut d_y) = self.damage_max {
            *d_y = (*d_y).max(y);
        } else {
            self.damage_max = Some(y);
        }
    }

    fn restore_viewport(&mut self, cursor: DrawCursor) {
        self.move_cursor_viewport(cursor.x, cursor.y);

        self.cursor = DrawCursor {
            x: self.cursor.x,
            y: self.cursor.y,
            ..cursor
        };
    }
    fn move_cursor_viewport(&mut self, mut x: u32, mut y: u32) {
        self.refresh_damage(false);
        let max_x = self.grid.width() - 1;
        let min_x = 0;

        let max_y = self.viewport_index + self.viewport_lines - 1;
        let min_y = self.viewport_index;

        x = x.clamp(min_x, max_x);
        y = y.clamp(min_y, max_y);

        self.cursor.x = x;
        self.cursor.y = y;
        self.refresh_damage(false);
    }
    fn move_cursor(&mut self, mut x: u32, mut y: u32) {
        self.refresh_damage(false);

        if x >= self.grid.width() {
            y += 1;
            x %= self.grid.width();
        }

        let to_scroll = y >= self.grid.height();
        while y >= self.grid.height() {
            self.grid
                .shift_up(&mut self.pixmap, LINE_HEIGHT.ceil() as u32);
            y -= 1;
        }

        let old_v = self.viewport_index;
        self.viewport_index = (y + 1).saturating_sub(self.viewport_lines);

        self.cursor.x = x;
        self.cursor.y = y;
        self.refresh_damage(old_v != self.viewport_index || to_scroll);
    }

    fn move_cursor_lines(&mut self, amount: i32) {
        let (x, y) = self.cursor();

        self.move_cursor_viewport(x, y.saturating_add_signed(amount));
    }

    fn move_cursor_chars(&mut self, amount: i32) {
        let (x, y) = self.cursor();

        self.move_cursor_viewport(x.saturating_add_signed(amount), y);
    }
}

impl vte::Perform for TermDisplay {
    fn print(&mut self, c: char) {
        self.insert_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0xa => self.insert_char('\n'),
            0x8 => self.insert_char('\x08'),
            0x0d => {
                let (_, y) = self.cursor();
                self.move_cursor_viewport(0, y);
            }
            0x09 => {
                self.insert_char('\t');
            }
            0x07 => {
                play_beep();
            }
            _ => println!("[execute] {:02x}", byte),
        }
    }

    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, c: char) {
        println!(
            "[hook] params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, c
        );
    }

    fn put(&mut self, byte: u8) {
        println!("[put] {:02x}", byte);
    }

    fn unhook(&mut self) {
        println!("[unhook]");
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        println!(
            "[osc_dispatch] params={:?} bell_terminated={}",
            params, bell_terminated
        );
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, c: char) {
        println!(
            "[csi_dispatch] params={:#?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, c
        );
        match c {
            'J' => {
                let mode = params.into_iter().next().map(|p| p[0]).unwrap_or(0);

                match mode {
                    0 => {
                        let (x, y) = self.cursor();
                        self.clear(x, self.grid.width(), y..self.grid.height(), false);
                    }
                    1 => {
                        let (x, y) = self.cursor();
                        self.clear(0, x + 1, 0..(y + 1), false);
                    }
                    2 => self.clear(0, self.grid.width(), 0..self.grid.height(), false),
                    3 => self.clear(0, self.grid.width(), 0..self.grid.height(), true),
                    _ => {}
                }
            }
            'H' => {
                let mut iter = params.into_iter();
                let y = (iter.next().unwrap_or(&[1])[0].max(1) - 1) as u32;
                let x = (iter.next().unwrap_or(&[1])[0].max(1) - 1) as u32;
                self.move_cursor_viewport(x, y)
            }
            'A' | 'B' | 'D' | 'C' => {
                let mut params = params.into_iter();
                let amount = params.next().unwrap_or(&[1])[0] as usize;

                match c {
                    'A' => self.move_cursor_lines(-(amount as i32)),
                    'B' => self.move_cursor_lines(amount as i32),
                    'C' => self.move_cursor_chars(amount as i32),
                    'D' => self.move_cursor_chars(-(amount as i32)),
                    _ => unreachable!(),
                }
            }

            'm' => {
                let params = params.into_iter();
                let mut params_single = params.map(|p| p.get(0).copied().unwrap_or_default());
                while let Some(param) = params_single.next() {
                    match param {
                        0 => {
                            self.reset();
                        }
                        1 => self.cursor.curr_weight = cosmic_text::Weight::BOLD,
                        22 => self.cursor.curr_weight = cosmic_text::Weight::default(),
                        color @ 30..=37
                        | color @ 40..=47
                        | color @ 90..=97
                        | color @ 100..=107
                        | color @ 39
                        | color @ 49 => {
                            let is_bright = color >= 90;
                            let is_bg = color > 39 && (color < 90 || color >= 100);

                            let generic = if is_bg && is_bright {
                                color - 100
                            } else if is_bg {
                                color - 40
                            } else if is_bright {
                                color - 90
                            } else {
                                color - 30
                            };

                            let pix = match (generic, is_bright) {
                                (0, false) => BLACK,
                                (0, true) => BRIGHT_BLACK,
                                (1, false) => RED,
                                (1, true) => BRIGHT_RED,
                                (2, false) => GREEN,
                                (2, true) => BRIGHT_GREEN,
                                (3, false) => YELLOW,
                                (3, true) => BRIGHT_YELLOW,
                                (4, false) => BLUE,
                                (4, true) => BRIGHT_BLUE,
                                (5, false) => MAGENTA,
                                (5, true) => BRIGHT_MAGENTA,
                                (6, false) => CYAN,
                                (6, true) => BRIGHT_CYAN,
                                (7, false) => WHITE,
                                (7, true) => BRIGHT_WHITE,
                                (9, false) if !is_bg => self.default_fg,
                                (9, false) if is_bg => Color::NONE,
                                (g, b) => {
                                    unreachable!("color value is {g}, bright: {b}, is_bg: {is_bg}")
                                }
                            };

                            if !is_bg {
                                self.cursor.curr_fg = pix;
                            } else {
                                self.cursor.curr_bg = pix;
                            }
                        }

                        v @ 38 | v @ 48 => match params_single.next() {
                            Some(2) => {
                                let is_bg = v == 48;
                                let r = params_single.next().unwrap_or_default() as u8;
                                let g = params_single.next().unwrap_or_default() as u8;
                                let b = params_single.next().unwrap_or_default() as u8;

                                let pix = Pixel::rgb(r, g, b);
                                if !is_bg {
                                    self.cursor.curr_fg = pix;
                                } else {
                                    self.cursor.curr_bg = pix;
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            'n' => {
                if let Some([n]) = params.iter().next() {
                    match n {
                        6 => {
                            self.requests
                                .push(TermRequest::CursorRequest(self.cursor()));
                            return;
                        }
                        _ => {}
                    }
                }
                println!(
                    "[csi_dispatch] params={:#?}, intermediates={:?}, ignore={:?}, char={:?}",
                    params, intermediates, ignore, c
                );
            }
            _ => println!(
                "[csi_dispatch] params={:#?}, intermediates={:?}, ignore={:?}, char={:?}",
                params, intermediates, ignore, c
            ),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            b'7' => {
                self.saved_cursor = self.cursor;
            }
            b'8' => {
                self.restore_viewport(self.saved_cursor);
            }
            _ => {
                println!(
                    "[esc_dispatch] intermediates={:?}, byte={:02x}",
                    intermediates, byte
                );
            }
        }
    }
}

impl<S, M> Shard<S, M> for TermDisplay {
    fn dirty(&self) -> bool {
        false
    }

    fn on_event(
        &mut self,
        event_ctx: &mut libgems::EventCtx,
        _: &libgems::ShardEvent,
        _: &mut Data<S, M>,
    ) {
        if let Some((p, a)) = self.collect_damage() {
            event_ctx.request_redraw_at(p, a);
        }
    }

    fn on_message(&mut self, ctx: &mut MsgCtx, _: &mut Data<S, M>, _: &M) {
        if let Some((p, a)) = self.collect_damage() {
            ctx.request_redraw_at(p, a);
        }
    }

    fn on_ctx_update(&mut self, ctx: &mut UpdateCtx, _: &Data<S, M>) {
        if let Some((p, a)) = self.collect_damage() {
            ctx.request_redraw_at(p, a);
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx) -> ShardLayout {
        let size = ctx.max_box();

        self.viewport_lines = (size.height() / LINE_HEIGHT).floor() as u32;
        ShardLayout {
            bounds: size,
            ..Default::default()
        }
    }

    fn render(&mut self, ctx: &mut libgems::shards::RenderCtx, _: &Data<S, M>) {
        let bounds = ctx.layout().bounds;
        let origin = ctx.origin();
        let damage_area = ctx.damage();

        let (render_point, render_area) = damage_area
            .intersection_with(origin, bounds)
            .expect("Failed to render terminal view because bad damage");

        let cur_y = (render_point.y() / LINE_HEIGHT as f32).ceil() as u32 + self.viewport_index;
        let cur_y_in_src =
            (render_point.y() + (self.viewport_index as f32 * LINE_HEIGHT)).floor() as u32;

        let m_cur_y = ((render_point.y() + render_area.height()) / LINE_HEIGHT as f32).ceil()
            as u32
            + self.viewport_index;

        for row_i in cur_y..m_cur_y {
            if row_i >= self.grid.height() {
                continue;
            }

            let row = &mut self.grid.rows[row_i as usize];
            let buf = row.prepare_data(ctx.font_system());
            if buf.redraw() {
                let target_y = (row_i as f32 * LINE_HEIGHT).ceil() as u32;
                let target_x = (0. * FONT_WIDTH).ceil() as u32;

                let canvas = ctx.canvas();

                let pixmap = &mut self.pixmap;
                let px_width = pixmap.width();

                let mut fill_rect_fast =
                    |x: u32,
                     y: u32,
                     width: u32,
                     height: u32,
                     blend: bool,
                     c: tiny_skia::PremultipliedColorU8| {
                        let pix_width = pixmap.width();

                        let pixels = pixmap.pixels_mut();
                        for h in 0..height {
                            if !blend {
                                let index = (((y + h) * pix_width) + x) as usize;
                                pixels[index..index + width as usize].fill(c);
                            } else {
                                for w in 0..width {
                                    let index = (((y + h) * pix_width) + x + w) as usize;
                                    utils::blend_pixel(&c, &mut pixels[index]);
                                }
                            }
                        }
                    };

                fill_rect_fast(
                    target_x,
                    target_y,
                    px_width,
                    LINE_HEIGHT.ceil() as u32,
                    false,
                    tiny_skia::PremultipliedColorU8::TRANSPARENT,
                );

                let cache = &mut canvas.cache.swash_cache;
                let font_system = &mut canvas.cache.font_system;

                for run in buf.layout_runs() {
                    let line_y = run.line_y;
                    let line_y_i32 = line_y as i32;

                    let line_top = run.line_top;
                    let line_height = run.line_height;

                    for glyph in run.glyphs.iter() {
                        let metadata = ExtraAttributes::from_usize(glyph.metadata);

                        if let Some(bg_color) = metadata.bg() {
                            fill_rect_fast(
                                glyph.x.floor() as u32,
                                target_y + line_top.floor() as u32,
                                FONT_WIDTH.ceil() as u32,
                                line_height.ceil() as u32,
                                false,
                                tiny_skia::PremultipliedColorU8::from_rgba(
                                    bg_color.r(),
                                    bg_color.g(),
                                    bg_color.b(),
                                    bg_color.a(),
                                )
                                .unwrap(),
                            );
                        }

                        if metadata.should_skip() {
                            continue;
                        }
                        let physical_glyph = glyph.physical((0., 0.), 1.0);
                        let glyph_color = match glyph.color_opt {
                            Some(some) => color_to_pix(some),
                            None => self.default_fg,
                        };

                        let p_x = physical_glyph.x;
                        let p_y = physical_glyph.y + line_y_i32 + target_y as i32;
                        cache.with_pixels(
                            font_system,
                            physical_glyph.cache_key,
                            pix_to_color(glyph_color),
                            |x, y, color| {
                                let x = (p_x + x) as u32;
                                let y = (p_y + y) as u32;

                                fill_rect_fast(
                                    x,
                                    y,
                                    1,
                                    1,
                                    true,
                                    tiny_skia::ColorU8::from_rgba(
                                        color.r(),
                                        color.g(),
                                        color.b(),
                                        color.a(),
                                    )
                                    .premultiply(),
                                );
                            },
                        );
                    }
                }

                buf.set_redraw(false);
            }
        }

        utils::tiny_blit_blend(
            ctx.pixmap(),
            self.pixmap.as_ref(),
            0,
            cur_y_in_src as i32,
            render_area.width().ceil() as i32,
            render_area.height().ceil() as i32,
            (origin.x() + render_point.x()).floor() as i32,
            (origin.y() + render_point.y()).floor() as i32,
        );

        let (c_x, c_y) = self.cursor();

        if c_y >= cur_y && c_y <= m_cur_y {
            let target_y = ((c_y - self.viewport_index) as f32 * LINE_HEIGHT) as f32 + origin.y();
            let target_x = (c_x as f32 * FONT_WIDTH) + origin.x();
            let transform = tiny_skia::Transform::from_translate(target_x, target_y);

            ctx.pixmap().fill_rect(
                Rect::from_xywh(0., 0., FONT_WIDTH as f32, LINE_HEIGHT as f32).unwrap(),
                PaintBrush::from(Color::WHITE)
                    .with_blend(tiny_skia::BlendMode::Difference)
                    .no_aa()
                    .as_paint(),
                transform,
                None,
            );
        }
    }
}

use Color as Pixel;
pub const BLACK: Pixel = Pixel::hex_rgb(0x282828);
pub const BRIGHT_BLACK: Pixel = Pixel::hex_rgb(0x928374);

pub const WHITE: Pixel = Pixel::hex_rgb(0xa89984);
pub const BRIGHT_WHITE: Pixel = Pixel::hex_rgb(0xebdbb2);

pub const RED: Pixel = Pixel::hex_rgb(0xcc241d);
pub const BRIGHT_RED: Pixel = Pixel::hex_rgb(0xfb4934);

pub const GREEN: Pixel = Pixel::hex_rgb(0x98971a);
pub const BRIGHT_GREEN: Pixel = Pixel::hex_rgb(0xb8bb26);

pub const BLUE: Pixel = Pixel::hex_rgb(0x458588);
pub const BRIGHT_BLUE: Pixel = Pixel::hex_rgb(0x83a598);

pub const YELLOW: Pixel = Pixel::hex_rgb(0xd79921);
pub const BRIGHT_YELLOW: Pixel = Pixel::hex_rgb(0xfabd2f);

pub const CYAN: Pixel = Pixel::hex_rgb(0x689d6a);
pub const BRIGHT_CYAN: Pixel = Pixel::hex_rgb(0x8ec07c);

pub const MAGENTA: Pixel = Pixel::hex_rgb(0xb16286);
pub const BRIGHT_MAGENTA: Pixel = Pixel::hex_rgb(0xd3869b);
