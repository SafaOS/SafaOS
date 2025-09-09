use std::sync::MutexGuard;

use libgem::{
    Gem,
    canvas::{DrawingCanvas, Pixel},
    cosmic_text::{
        Action, Attrs, AttrsList, Buffer, BufferRef, Color, Cursor, Edit, Editor, FontSystem,
        Metrics, SwashCache, Weight,
    },
    element::Element,
    text::{FONT_SYSTEM, SWASH_CACHE},
};

fn font_system() -> MutexGuard<'static, FontSystem> {
    FONT_SYSTEM
        .lock()
        .expect("Failed to acquire lock on FontSystem for console")
}

fn swash_cache() -> MutexGuard<'static, SwashCache> {
    SWASH_CACHE
        .lock()
        .expect("Failed to acquire lock on SwashCache for console")
}

const DEFAULT_WEIGHT: Weight = Weight::NORMAL;
const DEFAULT_TEXT_PIXEL: Pixel = Pixel::from_rgb(0xFF, 0xFF, 0xFF);
const DEFAULT_SELECTION_PIXEL: Pixel = Pixel::from_rgb_with_alpha(0xFF, 0xFF, 0xFF, 0x80);

/// Converts a pixel to a cosmic_text::Color
/// Pixel is premultiplied-alpha while a color is the opposite
const fn pix_to_color(pix: Pixel) -> Color {
    Color::rgba(pix.red(), pix.green(), pix.blue(), pix.alpha())
}

/// Converts a cosmic_text::color to a Pixel
/// Pixel is premultiplied-alpha while a color is the opposite
const fn color_to_pix(color: Color) -> Pixel {
    Pixel::from_hex_argb(color.0)
}
fn default_attrs() -> Attrs<'static> {
    Attrs::new()
        .color(pix_to_color(DEFAULT_TEXT_PIXEL))
        .weight(DEFAULT_WEIGHT)
}

pub struct ConsoleElement {
    editor: Editor<'static>,
    current_attr: Attrs<'static>,
    width: u32,
    height: u32,
    last_recorded_cursor: Option<(i32, i32)>,
}

impl ConsoleElement {
    pub fn new(width: u32, height: u32) -> Self {
        let mut font_system = font_system();

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 14.0));
        buffer.set_size(
            &mut font_system,
            Some(width as f32 - 12.),
            Some(height as f32 - (14. * 2.)),
        );
        let editor = Editor::new(buffer);
        Self {
            editor,
            width,
            height,
            current_attr: default_attrs(),
            last_recorded_cursor: None,
        }
    }

    fn first_idx(&self) -> Option<usize> {
        self.editor
            .with_buffer(|buf| buf.layout_runs().next().map(|s| s.line_i))
    }

    fn redraw_whole_screen(&self) -> ((usize, usize), (usize, usize)) {
        ((0, 0), (self.width as usize, self.height as usize))
    }

    pub fn insert_string(&mut self, str: &str) {
        self.editor
            .insert_string(str, Some(AttrsList::new(&self.current_attr)));
    }

    pub fn backspace(&mut self) {
        let mut font_system = font_system();
        self.editor.action(&mut font_system, Action::Backspace);
    }

    pub fn enter(&mut self) {
        let mut font_system = font_system();
        self.editor.action(&mut font_system, Action::Enter);
    }

    pub fn clear(&mut self) {
        self.editor
            .delete_range(Cursor::new(0, 0), self.editor.cursor());
    }

    pub fn get_cursor(&self) -> (usize, usize) {
        let cur = self.editor.cursor();
        (cur.index, cur.line)
    }

    pub fn set_cursor(&mut self, x: usize, y: usize) {
        self.editor.set_cursor(Cursor::new(x, y));
    }

    pub fn curr_height(&self) -> u32 {
        match self.editor.buffer_ref() {
            BufferRef::Owned(o) => {
                o.layout_runs().map(|l| l.line_height).sum::<f32>().ceil() as u32
            }
            _ => todo!(),
        }
    }

    pub fn curr_width(&self) -> u32 {
        self.editor.with_buffer(|buf| {
            buf.layout_runs()
                .map(|l| l.line_w)
                .max_by(|s, o| s.partial_cmp(o).unwrap_or(std::cmp::Ordering::Equal))
                .map(|f| f.ceil() as u32)
                .unwrap_or(0)
        })
    }

    /// Draw the editor ()
    #[allow(clippy::too_many_arguments)]
    fn draw_inner<F>(&self, draw_bounds: Option<((i32, i32), (i32, i32))>, mut f: F)
    where
        F: FnMut(i32, i32, u32, u32, Pixel),
    {
        const CURSOR_PIXEL: Pixel = Pixel::from_rgb(0xff, 0xff, 0xff);
        self.editor.draw(
            &mut font_system(),
            &mut swash_cache(),
            pix_to_color(DEFAULT_TEXT_PIXEL),
            pix_to_color(CURSOR_PIXEL),
            pix_to_color(DEFAULT_SELECTION_PIXEL),
            pix_to_color(DEFAULT_TEXT_PIXEL),
            |x, y, width, height, color| {
                if draw_bounds.is_none_or(|((min_x, min_y), (max_x, max_y))| {
                    x >= min_x && y >= min_y && x <= max_x && y <= max_y
                }) {
                    f(x, y, width, height, color_to_pix(color))
                }
            },
        );
    }
}

impl<Canvas: DrawingCanvas, G: Gem> Element<Canvas, G> for ConsoleElement {
    fn draw_height(&self) -> u32 {
        self.curr_height()
    }

    fn draw_width(&self) -> u32 {
        self.curr_width()
    }

    fn container_height(&self) -> u32 {
        self.height
    }

    fn container_width(&self) -> u32 {
        self.width
    }

    fn needs_redraw(&self) -> bool {
        self.editor.redraw()
    }

    fn draw(
        &mut self,
        canvas: &mut Canvas,
        start_x: u32,
        start_y: u32,
        bg_color: Pixel,
    ) -> Option<(u32, u32)> {
        let line_height = self
            .editor
            .with_buffer(|buf| buf.metrics().line_height.ceil() as i32);

        let old_first_line_idx = self.first_idx();
        let old_curr_pos = self.last_recorded_cursor;

        self.editor.shape_as_needed(&mut font_system(), false);

        let new_first_line_idx = self.first_idx();
        let new_curr_pos = self.editor.cursor_position();
        self.last_recorded_cursor = new_curr_pos;

        let didnt_scroll = new_first_line_idx == old_first_line_idx;
        let redraw_bounds = didnt_scroll.then(|| {
            let (s_x, s_y) = old_curr_pos.unwrap_or_default();
            let (e_x, e_y) = new_curr_pos.unwrap_or((self.width as i32, self.height as i32));

            let min_y = s_y.min(e_y);
            let max_y = e_y.max(s_y);
            let min_x = s_x.min(e_x);
            let max_x = e_x.max(s_x);

            // If the ys aren't equal, we moved a few lines down so the max X should be a line worth
            let max_x = if min_y == max_y {
                max_x + 1 /* include cursor */
            } else {
                self.width as i32
            };
            // Include the cursor
            let max_y = max_y + line_height /* include cursor */;

            ((min_x, min_y), (max_x, max_y))
        });

        let (c_start_x, c_start_y, c_width, c_height) = redraw_bounds
            .map(|((min_x, min_y), (max_x, max_y))| {
                (
                    start_x.saturating_add_signed(min_x),
                    start_y.saturating_add_signed(min_y),
                    (max_x - min_x) as u32,
                    (max_y - min_y) as u32,
                )
            })
            .unwrap_or((start_x, start_y, self.width, self.height));

        canvas.draw_rect(
            c_start_x,
            c_start_y,
            c_width,
            c_height,
            Pixel::from_hex_argb(0x0),
            Some(bg_color),
        );

        self.draw_inner(redraw_bounds, |x, y, w, h, pixel| {
            if x.is_negative() || y.is_negative() {
                return;
            }

            let draw_x = start_x.saturating_add(x as u32);
            let draw_y = start_y.saturating_add(y as u32);
            canvas.draw_rect(draw_x, draw_y, w, h, pixel, Some(bg_color));
        });

        self.editor.set_redraw(false);
        Some((c_start_x + c_width, c_start_y + c_height))
    }
}

impl vte::Perform for ConsoleElement {
    fn print(&mut self, c: char) {
        let mut tmp = [0u8; 4];
        let s = c.encode_utf8(&mut tmp);
        self.insert_string(s);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0xa => self.enter(),
            0x8 => self.backspace(),
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
        match c {
            'J' => self.clear(),
            'H' => {
                let mut iter = params.into_iter();
                let x = iter.next().unwrap_or(&[0])[0] as usize;
                let y = iter.next().unwrap_or(&[0])[0] as usize;
                self.set_cursor(x, y)
            }
            'm' => {
                let params = params.into_iter();
                let mut params_single = params.map(|p| p.get(0).copied().unwrap_or_default());
                match params_single.next() {
                    Some(0) => {
                        // reset all
                        self.current_attr = default_attrs();
                    }
                    Some(1) => self.current_attr = self.current_attr.clone().weight(Weight::BOLD),
                    Some(22) => {
                        self.current_attr = self.current_attr.clone().weight(default_attrs().weight)
                    }
                    _ => {}
                }
            }
            _ => println!(
                "[csi_dispatch] params={:#?}, intermediates={:?}, ignore={:?}, char={:?}",
                params, intermediates, ignore, c
            ),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        println!(
            "[esc_dispatch] intermediates={:?}, ignore={:?}, byte={:02x}",
            intermediates, ignore, byte
        );
    }
}
