use bitflags::bitflags;
use core::fmt::Write;
use framebuffer::FrameBufferTTY;
use lazy_static::lazy_static;
use spin::RwLock;

use crate::{
    drivers::keyboard::{
        keys::{Key, KeyCode, KeyFlags},
        HandleKey,
    },
    eve::KERNEL_ABI_STRUCTURES,
    threading::expose::{pspawn, SpawnFlags},
    utils::{alloc::PageBString, bstr::BStr},
};
use safa_utils::{make_path, types::Name};

pub mod framebuffer;

/// defines the interface for a tty
/// a tty is a user-visible device that can be written to, and that user-input can be read from
/// it is recommended for the tty to support ansii escape sequences, some stuff will be managed by a
/// higher-level tty implementation `TTY` only writing to the tty is required
pub trait TTYInterface: Send + Sync {
    fn write_str(&mut self, s: &BStr);

    fn draw_cursor(&mut self);
    fn hide_cursor(&mut self);
    /// sets the cursor to x y
    /// which are in characters
    fn set_cursor(&mut self, x: usize, y: usize);
    /// set the cursor to cursor x + `x`, cursor y + `y`
    fn offset_cursor(&mut self, x: isize, y: isize);
    /// sets the cursor to the beginning of a new line
    fn newline(&mut self);
    /// scrolls the screen down
    /// does not move the cursor
    fn scroll_down(&mut self);
    /// scrolls the screen up
    /// does not move the cursor
    fn scroll_up(&mut self);
    /// clears the screen
    /// does not move the cursor
    fn clear(&mut self);
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct TTYSettings: u8 {
        /// whether or not we are currently receiving input
        /// the cursor should work well if enabled correctly using `self.enable_input` and disabled
        /// using `self.disable_input`
        // TODO: maybe the cursor should be the job of the shell?
        const RECEIVE_INPUT = 1 << 0;
        const DRAW_GRAPHICS = 1 << 1;
        const CANONICAL_MODE = 1 << 2;
        const ECHO_INPUT = 1 << 3;
    }
}

struct Stdin {
    inner: heapless::Vec<u8, 1024>,
    cursor: usize,
}

impl Stdin {
    const fn new() -> Self {
        Self {
            inner: heapless::Vec::new(),
            cursor: 0,
        }
    }

    #[inline(always)]
    /// Writes `s.len()` bytes at `offset`, returns the amount of bytes written, writtes the results to a given `stdout` at the end
    fn write_at(&mut self, offset: usize, s: &[u8], stdout: Option<&mut PageBString>) -> usize {
        assert!(offset <= self.inner.len());
        let amount = s.len();
        if amount > self.inner.capacity() - self.inner.len() || amount == 0 {
            return 0;
        }

        if offset == self.inner.len() {
            let old_len = self.inner.len();
            let new_len = old_len + amount;

            unsafe {
                self.inner.set_len(new_len);
            }
            self.inner[old_len..].copy_from_slice(s);

            if let Some(stdout) = stdout {
                stdout.push_bytes(s);
            }
            return amount;
        }

        let prev_len = self.inner.len();
        let new_len = prev_len + amount;
        unsafe {
            self.inner.set_len(new_len);
        }

        let at_offset = &mut self.inner[offset..];
        // first we want to shift all elements at offset by amount then we replace the free space with our slice
        at_offset.copy_within(..at_offset.len() - amount, amount);
        at_offset[..amount].copy_from_slice(&s);

        if let Some(stdout) = stdout {
            // go back to offset and rewrite it from the start
            stdout.push_bytes(&at_offset);
            _ = stdout.write_fmt(format_args!("\x1b[{}D", at_offset.len() - 1));
        }
        return amount;
    }

    /// writes the given string to the buffer at the current cursor position and updates the stdout buffer if given
    #[inline(always)]
    fn write(&mut self, s: &str, stdout: Option<&mut PageBString>) {
        let amount = self.write_at(self.cursor, s.as_bytes(), stdout);
        self.cursor += amount;
    }

    // tries to increase the cursor position to the right
    // returns true if successful
    fn cursor_right(&mut self) -> bool {
        let bytes = self.inner.as_slice();
        if self.cursor >= bytes.len() {
            return false;
        };
        self.cursor += 1;
        true
    }

    // tries to decreasse the cursor position to the left
    // returns true if successful
    fn cursor_left(&mut self) -> bool {
        if self.cursor == 0 {
            false
        } else {
            self.cursor -= 1;
            true
        }
    }

    #[inline(always)]
    pub fn as_str(&self) -> &BStr {
        BStr::from_bytes(&self.inner)
    }

    #[inline(always)]
    pub fn pop_front(&mut self, amount: usize) {
        self.inner.copy_within(amount.., 0);

        let old_len = self.inner.len();
        unsafe {
            self.inner.set_len(old_len - amount);
        }
        self.cursor -= amount;
    }

    #[inline(always)]
    pub fn pop(&mut self) -> bool {
        self.inner.pop().is_some()
    }

    #[inline(always)]
    // pops a character at the cursor position and updates the stdout buffer if given
    pub fn pop_at_cursor(&mut self, stdout: Option<&mut PageBString>) {
        if self.cursor == 0 {
            return;
        }

        if self.cursor >= self.inner.len() {
            if self.pop() {
                self.cursor -= 1;
                if let Some(stdout) = stdout {
                    // go back a character draw a space and go back again
                    // FIXME: this is a hack to remove a character from the screen
                    stdout.push_str("\x1b[1D \x1b[1D");
                }
            }

            return;
        }

        let old_cursor = self.cursor;
        if self.cursor_left() {
            let new_cursor = self.cursor;
            let bytes_moved = old_cursor - new_cursor;

            self.inner.copy_within(old_cursor.., new_cursor);
            unsafe {
                self.inner.set_len(self.inner.len() - bytes_moved);
            }

            if let Some(stdout) = stdout {
                let bytes = &self.inner[new_cursor..];
                // go back a character
                stdout.push_str("\x1b[1D");
                // write the characters that comes after the removed character
                stdout.push_bytes(bytes);

                // FIXME: this is a hack to remove a character from the screen
                stdout.push_char(' ');

                // go back to original cursor position
                _ = stdout.write_fmt(format_args!("\x1b[{}D", bytes.len() + 1));
            }
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
pub struct TTY<T: TTYInterface> {
    /// stores the stdout buffer for write operations performed on the tty device, allows to write to the tty at once instead of a piece by piece
    stdout_buffer: PageBString,
    /// stores the stdin buffer for read operations performed on the tty device
    stdin: Stdin,
    pub settings: TTYSettings,
    interface: T,
    cursor_visible: bool,
}

impl<T: TTYInterface> Write for TTY<T> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_bstr(s.into());
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.stdout_buffer.push_char(c);
        Ok(())
    }

    fn write_fmt(&mut self, args: core::fmt::Arguments<'_>) -> core::fmt::Result {
        if let Some(s) = args.as_str() {
            _ = self.write_str(s);
            self.sync();
        } else {
            self.stdout_buffer.write_fmt(args)?;
            self.sync();
        }
        Ok(())
    }
}

impl<T: TTYInterface> TTY<T> {
    pub fn new(interface: T) -> Self {
        Self {
            stdin: Stdin::new(),
            stdout_buffer: PageBString::with_capacity(4096),
            interface,
            settings: TTYSettings::DRAW_GRAPHICS
                | TTYSettings::CANONICAL_MODE
                | TTYSettings::ECHO_INPUT,
            cursor_visible: false,
        }
    }

    pub fn clear(&mut self) {
        self.interface.clear();
        self.interface.set_cursor(0, 0);
    }

    #[inline(always)]
    pub fn hide_cursor(&mut self) {
        if !self.cursor_visible {
            return;
        }

        self.cursor_visible = false;
        self.interface.hide_cursor();
    }

    #[inline(always)]
    pub fn show_cursor(&mut self) {
        if self.cursor_visible {
            return;
        }

        self.cursor_visible = true;
        self.interface.draw_cursor();
    }

    pub fn enable_input(&mut self) {
        if !self.settings.contains(TTYSettings::RECEIVE_INPUT) {
            self.settings |= TTYSettings::RECEIVE_INPUT;
            self.show_cursor();
        }
    }

    pub fn disable_input(&mut self) {
        if self.settings.contains(TTYSettings::RECEIVE_INPUT) {
            self.settings &= !TTYSettings::RECEIVE_INPUT;
            self.hide_cursor();
        }
    }

    pub fn perform_backspace(&mut self) {
        if !self.stdin().is_empty() {
            // backspace
            self.stdin.pop_at_cursor(Some(&mut self.stdout_buffer));
            self.sync();
        }
    }

    /// syncs the buffer by actually writing it to the interface
    pub fn sync(&mut self) {
        if self.settings.contains(TTYSettings::DRAW_GRAPHICS)
            | self.settings.contains(TTYSettings::ECHO_INPUT)
        {
            let cursor_was_visible = self.cursor_visible;
            self.hide_cursor();

            self.interface.write_str(self.stdout_buffer.as_bstr());
            self.stdout_buffer.clear();

            if cursor_was_visible {
                self.show_cursor();
            }
        }
    }

    pub fn write_bstr(&mut self, s: &BStr) {
        self.stdout_buffer.push_bstr(s);
    }

    #[inline]
    pub fn stdin(&self) -> &BStr {
        self.stdin.as_str()
    }

    #[inline(always)]
    pub fn stdin_pop_front(&mut self, amount: usize) {
        self.stdin.pop_front(amount)
    }

    #[inline(always)]
    const fn is_interactive(&self) -> bool {
        self.settings.contains(TTYSettings::RECEIVE_INPUT)
            && self.settings.contains(TTYSettings::CANONICAL_MODE)
    }
}

lazy_static! {
    pub static ref FRAMEBUFFER_TERMINAL: RwLock<TTY<FrameBufferTTY<'static>>> =
        RwLock::new(TTY::new(FrameBufferTTY::new()));
}

impl<T: TTYInterface> HandleKey for TTY<T> {
    fn handle_key(&mut self, key: Key) {
        macro_rules! write_key {
            ($mapped: expr) => {
                if self.settings.contains(TTYSettings::ECHO_INPUT) {
                    self.interface.hide_cursor();
                    let _ = self.interface.write_str($mapped.into());
                    self.interface.draw_cursor();
                }
            };
            () => {
                let mapped = key.map_key();
                write_key!(mapped)
            };
        }
        match key.code {
            KeyCode::PageDown => self.interface.scroll_down(),
            KeyCode::PageUp => self.interface.scroll_up(),
            KeyCode::KeyC if key.flags.contains(KeyFlags::CTRL | KeyFlags::SHIFT) => {
                self.clear();
                self.interface.set_cursor(1, 1);
                pspawn(
                    Name::try_from("Shell").unwrap(),
                    // Maybe we can make a const function or a macro for this
                    make_path!("sys", "bin/safa"),
                    &["sys:/bin/safa", "-i"],
                    &[b"PATH=sys:/bin", b"SHELL=sys:/bin/safa"],
                    SpawnFlags::empty(),
                    *KERNEL_ABI_STRUCTURES,
                )
                .unwrap();
            }
            KeyCode::Backspace if self.is_interactive() => {
                self.hide_cursor();
                self.perform_backspace();
                self.show_cursor();
            }

            KeyCode::Left if self.is_interactive() => {
                if self.stdin.cursor_left() {
                    write_key!();
                }
            }
            KeyCode::Right if self.is_interactive() => {
                if self.stdin.cursor_right() {
                    write_key!();
                }
            }
            _ if self.settings.contains(TTYSettings::RECEIVE_INPUT) => {
                let mapped = key.map_key();
                if mapped.is_empty() {
                    return;
                }

                let stdout = if self.settings.contains(TTYSettings::ECHO_INPUT) {
                    Some(&mut self.stdout_buffer)
                } else {
                    None
                };

                self.stdin.write(mapped, stdout);
                self.sync();
            }

            _ => {}
        }
    }
}

/// writes to the framebuffer terminal
#[doc(hidden)]
#[unsafe(no_mangle)]
pub fn _print(args: core::fmt::Arguments) {
    unsafe {
        FRAMEBUFFER_TERMINAL
            .write()
            .write_fmt(args)
            .unwrap_unchecked();
    }
}
