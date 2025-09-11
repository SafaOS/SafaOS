use std::sync::MutexGuard;

use libgem::{
    Gem,
    canvas::{DrawingCanvas, Pixel},
    cosmic_text::{
        Attrs, AttrsList, Buffer, BufferLine, Color, Cursor, FontSystem, LayoutRun, Metrics,
        Shaping, SwashCache, Weight,
    },
    element::Element,
    text::{FONT_SYSTEM, SWASH_CACHE},
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

    /// Returns the background color of the cell containing the glyph or None if the background is transparent
    pub const fn bg(&self) -> Option<Pixel> {
        if unsafe { core::mem::transmute::<Pixel, u32>(self.background_color) } != 0 {
            Some(self.background_color)
        } else {
            None
        }
    }

    /// Whether or not we can skip rendering this glyph
    pub const fn should_skip(&self) -> bool {
        unsafe { core::mem::transmute::<_, usize>(self) == 1 }
    }

    pub const fn new_skip_rendering() -> Self {
        unsafe { core::mem::transmute(1usize) }
    }

    pub const fn with_bg(mut self, bg: Pixel) -> Self {
        self.background_color = bg;
        self
    }
}

const _: () = assert!(size_of::<ExtraAttributes>() == size_of::<usize>());

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

/// Stolen from [`libgem::cosmic_text::Editor`]
fn cursor_glyph_opt(cursor: &Cursor, run: &LayoutRun) -> Option<(usize, f32)> {
    use unicode_segmentation::UnicodeSegmentation;

    if cursor.line == run.line_i {
        for (glyph_i, glyph) in run.glyphs.iter().enumerate() {
            if cursor.index == glyph.start {
                return Some((glyph_i, 0.0));
            } else if cursor.index > glyph.start && cursor.index < glyph.end {
                // Guess x offset based on characters
                let mut before = 0;
                let mut total = 0;

                let cluster = &run.text[glyph.start..glyph.end];
                for (i, _) in cluster.grapheme_indices(true) {
                    if glyph.start + i < cursor.index {
                        before += 1;
                    }
                    total += 1;
                }

                let offset = glyph.w * (before as f32) / (total as f32);
                return Some((glyph_i, offset));
            }
        }
        match run.glyphs.last() {
            Some(glyph) => {
                if cursor.index == glyph.end {
                    return Some((run.glyphs.len(), 0.0));
                }
            }
            None => {
                return Some((0, 0.0));
            }
        }
    }
    None
}

/// Stolen from [`libgem::cosmic_text::Editor`], returns the cooridations of the cursor if it is visible.
fn cursor_position(cursor: &Cursor, run: &LayoutRun) -> Option<(i32, i32)> {
    let (cursor_glyph, cursor_glyph_offset) = cursor_glyph_opt(cursor, run)?;
    let x = match run.glyphs.get(cursor_glyph) {
        Some(glyph) => {
            // Start of detected glyph
            if glyph.level.is_rtl() {
                (glyph.x + glyph.w - cursor_glyph_offset) as i32
            } else {
                (glyph.x + cursor_glyph_offset) as i32
            }
        }
        None => match run.glyphs.last() {
            Some(glyph) => {
                // End of last glyph
                if glyph.level.is_rtl() {
                    glyph.x as i32
                } else {
                    (glyph.x + glyph.w) as i32
                }
            }
            None => {
                // Start of empty line
                0
            }
        },
    };

    Some((x, run.line_top as i32))
}

const DEFAULT_WEIGHT: Weight = Weight::NORMAL;
const DEFAULT_TEXT_PIXEL: Pixel = Pixel::from_rgb(0xFF, 0xFF, 0xFF);
const CURSOR_WIDTH: u32 = 2;
const CURSOR_COLOR_PIX: Pixel = DEFAULT_TEXT_PIXEL;
const FONT_HEIGHT: u32 = 12;
const FONT_WIDTH: u32 = 8;
const CHAR_AMOUNT_LIMIT: usize = 400;
const MAX_HISTORY_LINES: usize = 200;

pub const BLACK: Pixel = Pixel::from_hex_rgb(0x282828);
pub const BRIGHT_BLACK: Pixel = Pixel::from_hex_rgb(0x928374);

pub const WHITE: Pixel = Pixel::from_hex_rgb(0xa89984);
pub const BRIGHT_WHITE: Pixel = Pixel::from_hex_rgb(0xebdbb2);

pub const RED: Pixel = Pixel::from_hex_rgb(0xcc241d);
pub const BRIGHT_RED: Pixel = Pixel::from_hex_rgb(0xfb4934);

pub const GREEN: Pixel = Pixel::from_hex_rgb(0x98971a);
pub const BRIGHT_GREEN: Pixel = Pixel::from_hex_rgb(0xb8bb26);

pub const BLUE: Pixel = Pixel::from_hex_rgb(0x458588);
pub const BRIGHT_BLUE: Pixel = Pixel::from_hex_rgb(0x83a598);

pub const YELLOW: Pixel = Pixel::from_hex_rgb(0xd79921);
pub const BRIGHT_YELLOW: Pixel = Pixel::from_hex_rgb(0xfabd2f);

pub const CYAN: Pixel = Pixel::from_hex_rgb(0x689d6a);
pub const BRIGHT_CYAN: Pixel = Pixel::from_hex_rgb(0x8ec07c);

pub const MAGENTA: Pixel = Pixel::from_hex_rgb(0xb16286);
pub const BRIGHT_MAGENTA: Pixel = Pixel::from_hex_rgb(0xd3869b);

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

pub struct TerminalElement {
    buffer: Buffer,
    current_attr: Attrs<'static>,
    width: u32,
    height: u32,
    last_recorded_cursor_pos: Option<(i32, i32)>,
    cursor: Cursor,
}

impl TerminalElement {
    pub fn new(width: u32, height: u32) -> Self {
        let mut font_system = font_system();

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(FONT_HEIGHT as f32, 14.0));
        buffer.set_size(
            &mut font_system,
            Some(width as f32 - FONT_HEIGHT as f32),
            Some(height as f32 - (14. * 2.)),
        );
        buffer.set_monospace_width(&mut font_system, Some(FONT_WIDTH as f32));

        Self {
            buffer,
            width,
            height,
            current_attr: default_attrs(),
            last_recorded_cursor_pos: None,
            cursor: Cursor::new(0, 0),
        }
    }

    fn first_idx(&self) -> Option<usize> {
        self.buffer.layout_runs().next().map(|s| s.line_i)
    }

    pub fn insert_char(&mut self, c: char) {
        // replace the cursor
        let new_cursor = self.insert_char_at(self.cursor, c);

        // the new cursor
        // We need to insert it to get the position of it as a glyph...
        self.cursor = new_cursor;

        self.buffer.set_redraw(true);
    }

    /// Returns the position of the cursor as a glyph if it exist
    fn cursor_position(&self) -> Option<(i32, i32)> {
        self.buffer
            .layout_runs()
            .find_map(|run| cursor_position(&self.cursor, &run))
    }

    fn max_chars_len(&self) -> usize {
        ((self.width / FONT_WIDTH) as usize).min(CHAR_AMOUNT_LIMIT)
    }

    /// Insert a character at cursor `cursor`, returns the new cursor position, also handles both newline '\n' and backspace '\x08'.
    fn insert_char_at(&mut self, cursor: Cursor, data: char) -> Cursor {
        let max_char_len = self.max_chars_len();

        let mut curr_line = cursor.line;
        let mut curr_col = cursor.index;

        let buf = &mut self.buffer;
        while curr_col >= max_char_len {
            curr_col -= max_char_len;
            curr_line += 1;
        }

        while curr_line >= buf.lines.len() {
            let ending = buf.lines.last().map(|l| l.ending()).unwrap_or_default();

            if buf.lines.len() + 1 >= MAX_HISTORY_LINES {
                unsafe {
                    // We drop the first line which we are going to remove
                    std::ptr::drop_in_place(buf.lines.as_mut_ptr());
                    // We copy from the second line to the end, to take place of the first line
                    std::ptr::copy(
                        buf.lines.as_ptr().add(1),
                        buf.lines.as_mut_ptr(),
                        buf.lines.len() - 1,
                    );
                    // We adjust length
                    buf.lines.set_len(buf.lines.len() - 1);
                }
                curr_line -= 1;
            }
            buf.lines.push(BufferLine::new(
                String::new(),
                ending,
                AttrsList::new(&self.current_attr),
                Shaping::Advanced,
            ));
        }

        if data == '\x08' {
            if let Some(prev_col) = curr_col.checked_sub(1) {
                // go back a single column
                curr_col = prev_col;
            } else {
                return Cursor::new(curr_line, 0);
            }
        } else if data == '\n' {
            let last_line = buf
                .lines
                .last_mut()
                .expect("curr_line lines should be created before this.");

            let ending = last_line.ending();
            let col_to_indx = last_line
                .text()
                .char_indices()
                .enumerate()
                .map(|(i, (col, _))| (i, col))
                .find(|(_, col)| *col == curr_col)
                .map(|(i, _)| i);

            // if the \n interrupts an existing position
            return if let Some(idx) = col_to_indx {
                let new_line = last_line.split_off(idx);
                buf.lines.push(new_line);
                Cursor::new(curr_line + 1, 0)
            } else {
                // String::from(" ") to make sure the cursor is valid
                buf.lines.push(BufferLine::new(
                    String::from(" "),
                    ending,
                    AttrsList::new(&self.current_attr),
                    Shaping::Advanced,
                ));
                Cursor::new(curr_line + 1, 0)
            };
        }

        let line_ref = buf.lines.get_mut(curr_line as usize).unwrap();
        let line: BufferLine = std::mem::replace(
            line_ref,
            BufferLine::new(
                String::new(),
                Default::default(),
                AttrsList::new(&Attrs::new()),
                Shaping::Basic,
            ),
        );

        let ending = line.ending();
        let mut attr_list = line.attrs_list().clone();
        if data != '\x08' {
            attr_list.add_span(curr_col as usize..curr_col as usize + 1, &self.current_attr);
        }

        let line_text = line.into_text();

        let mut result_string: arrayvec::ArrayString<{ CHAR_AMOUNT_LIMIT * 4 }> =
            arrayvec::ArrayString::new_const();

        let mut last_col = None;
        let mut inserted = false;

        for (_, (col, c)) in line_text.char_indices().enumerate() {
            last_col = Some(col);

            if col == curr_col {
                if data != '\x08'
                /* backspace just removes a single character */
                {
                    result_string.push(data);
                }
                inserted = true;
                break;
            } else {
                result_string.push(c);
            }
        }

        assert!(last_col.is_some() || line_text.len() <= 0);
        if !inserted && data != '\x08' {
            let cols_missing = last_col.map(|l| curr_col - l - 1).unwrap_or(curr_col);
            if cols_missing != 0 {
                for _ in 0..cols_missing {
                    result_string.push(' ');
                }
                let start_col = last_col.unwrap_or_default();
                // Skip rendering of spaces
                attr_list.add_span(
                    start_col..curr_col,
                    &Attrs::new().metadata(ExtraAttributes::new_skip_rendering().raw()),
                );
            }

            result_string.push(data);
        }

        let mut new_string = line_text;
        new_string.clear();
        new_string.insert_str(0, &result_string);

        *line_ref = BufferLine::new(new_string, ending, attr_list, Shaping::Advanced);
        line_ref.reset();

        if data == '\x08' {
            Cursor::new(curr_line, curr_col /* we went back a col remember */)
        } else if curr_col + 1 < max_char_len {
            Cursor::new(curr_line, curr_col + 1)
        } else {
            Cursor::new(curr_line + 1, 0)
        }
    }

    #[inline(always)]
    pub fn move_cursor_lines(&mut self, lines: i32) {
        if lines == 0 {
            return;
        }

        self.cursor = Cursor::new(
            self.cursor.line.saturating_add_signed(lines as isize),
            self.cursor.index,
        );
        self.buffer.set_redraw(true);
    }

    #[inline(always)]
    pub fn move_cursor_chars(&mut self, amount: i32) {
        if amount == 0 {
            return;
        }

        let max_chars = self.max_chars_len();
        self.cursor = Cursor::new(
            self.cursor.line,
            self.cursor
                .index
                .saturating_add_signed(amount as isize)
                .min(max_chars - 1),
        );

        self.buffer.set_redraw(true);
    }

    pub fn backspace(&mut self) {
        self.insert_char('\x08');
    }

    pub fn enter(&mut self) {
        self.insert_char('\n');
    }

    pub fn clear(&mut self) {
        self.buffer.lines.clear();
        self.buffer.set_redraw(true);
    }

    pub fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor = Cursor::new(y, x);
        self.buffer.set_redraw(true);
    }

    pub fn curr_height(&self) -> u32 {
        self.buffer
            .layout_runs()
            .map(|l| l.line_height)
            .sum::<f32>()
            .ceil() as u32
    }

    pub fn curr_width(&self) -> u32 {
        self.buffer
            .layout_runs()
            .map(|l| l.line_w)
            .max_by(|s, o| s.partial_cmp(o).unwrap_or(std::cmp::Ordering::Equal))
            .map(|f| f.ceil() as u32)
            .unwrap_or(0)
    }

    /// Draw the editor ()
    #[allow(clippy::too_many_arguments)]
    fn draw_inner<F>(
        &self,
        cursor_pos: Option<(i32, i32)>,
        draw_bounds: Option<((i32, i32), (i32, i32))>,
        mut f: F,
    ) where
        F: FnMut(i32, i32, u32, u32, Pixel),
    {
        let buf = &self.buffer;
        let line_height = buf.metrics().line_height;

        let text_color = pix_to_color(DEFAULT_TEXT_PIXEL);

        let cache = &mut swash_cache();
        let font_system = &mut font_system();

        for run in buf.layout_runs() {
            let line_y = run.line_y;
            let line_y_i32 = line_y as i32;

            if draw_bounds
                .is_some_and(|((_, min_y), (_, max_y))| line_y_i32 < min_y || line_y_i32 > max_y)
            {
                continue;
            }

            let run_glyphs = run
                .glyphs
                .iter()
                .map(|gly| (gly, gly.physical((0., 0.), 1.0)));

            for (glyph, physical_glyph) in run_glyphs {
                let metadata = ExtraAttributes::from_usize(glyph.metadata);
                if metadata.should_skip()
                    || draw_bounds.is_some_and(|((min_x, min_y), (max_x, max_y))| {
                        physical_glyph.x < min_x
                            || (physical_glyph.y + line_y_i32) < min_y
                            || physical_glyph.x > max_x
                            || (physical_glyph.y + line_y_i32) > max_y
                    })
                {
                    continue;
                }

                if let Some(bg_color) = metadata.bg() {
                    f(
                        glyph.x as i32,
                        run.line_top as i32,
                        FONT_WIDTH,
                        line_height as u32,
                        bg_color,
                    )
                }

                let glyph_color = match glyph.color_opt {
                    Some(some) => some,
                    None => text_color,
                };

                cache.with_pixels(
                    font_system,
                    physical_glyph.cache_key,
                    glyph_color,
                    |x, y, color| {
                        f(
                            physical_glyph.x + x,
                            line_y as i32 + physical_glyph.y + y,
                            1,
                            1,
                            color_to_pix(color),
                        );
                    },
                );
            }
        }
        // Draw cursor
        if let Some((x, y)) = cursor_pos {
            f(x, y, CURSOR_WIDTH, line_height as u32, CURSOR_COLOR_PIX);
        }
    }
}

impl<Canvas: DrawingCanvas, G: Gem> Element<Canvas, G> for TerminalElement {
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
        self.buffer.redraw()
    }

    fn draw(
        &mut self,
        canvas: &mut Canvas,
        start_x: u32,
        start_y: u32,
        bg_color: Pixel,
    ) -> Option<(u32, u32)> {
        let line_height = self.buffer.metrics().line_height.ceil() as i32;

        let old_first_line_idx = self.first_idx();
        let old_curr_pos = self.last_recorded_cursor_pos;

        self.buffer
            .shape_until_cursor(&mut font_system(), self.cursor, false);

        let new_first_line_idx = self.first_idx();
        let new_curr_pos = self.cursor_position();
        self.last_recorded_cursor_pos = new_curr_pos;

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
                    (max_x - min_x + 1) as u32,
                    (max_y - min_y + 1) as u32,
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

        self.draw_inner(new_curr_pos, redraw_bounds, |x, y, w, h, pixel| {
            if x.is_negative() || y.is_negative() {
                return;
            }

            let draw_x = start_x.saturating_add(x as u32);
            let draw_y = start_y.saturating_add(y as u32);
            canvas.draw_rect(draw_x, draw_y, w, h, pixel, None);
        });

        self.buffer.set_redraw(false);
        Some((c_start_x + c_width, c_start_y + c_height))
    }
}

impl vte::Perform for TerminalElement {
    fn print(&mut self, c: char) {
        self.insert_char(c);
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
        println!(
            "[csi_dispatch] params={:#?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, c
        );
        match c {
            'J' => self.clear(),
            'H' => {
                let mut iter = params.into_iter();
                let x = iter.next().unwrap_or(&[0])[0] as usize;
                let y = iter.next().unwrap_or(&[0])[0] as usize;
                self.set_cursor(x, y)
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
                            // reset all
                            self.current_attr = default_attrs();
                        }
                        1 => self.current_attr.weight = Weight::BOLD,
                        22 => self.current_attr.weight = default_attrs().weight,
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
                                (9, false) if !is_bg => DEFAULT_TEXT_PIXEL,
                                (9, false) if is_bg => Pixel::from_hex_argb(0),
                                (g, b) => {
                                    unreachable!("color value is {g}, bright: {b}, is_bg: {is_bg}")
                                }
                            };

                            if !is_bg {
                                self.current_attr.color_opt = Some(pix_to_color(pix))
                            } else {
                                self.current_attr.metadata =
                                    ExtraAttributes::from_usize(self.current_attr.metadata)
                                        .with_bg(pix)
                                        .raw()
                            }
                        }

                        v @ 38 | v @ 48 => match params_single.next() {
                            Some(2) => {
                                let is_bg = v == 48;
                                let r = params_single.next().unwrap_or_default() as u8;
                                let g = params_single.next().unwrap_or_default() as u8;
                                let b = params_single.next().unwrap_or_default() as u8;

                                let pix = Pixel::from_rgb(r, g, b);
                                let color = Color::rgb(r, g, b);

                                if !is_bg {
                                    self.current_attr.color_opt = Some(color)
                                } else {
                                    self.current_attr.metadata =
                                        ExtraAttributes::from_usize(self.current_attr.metadata)
                                            .with_bg(pix)
                                            .raw()
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
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
