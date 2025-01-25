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
    threading::expose::{pspawn, SpawnFlags},
    utils::alloc::PageString,
};

pub mod framebuffer;

/// defines the interface for a tty
/// a tty is a user-visible device that can be written to, and that user-input can be read from
/// it is recommened for the tty to support ansii escape sequences, some stuff will be managed by a
/// higher-level tty implementation `TTY` only writing to the tty is required
pub trait TTYInterface: Send + Sync + Write {
    /// removes the character at the current cursor position
    /// and moves the cursor to the left
    fn backspace(&mut self);
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
        /// wether or not we are currently reciving input
        /// the cursor should work well if enabled correctly using `self.enable_input` and disabled
        /// using `self.disable_input`
        // TODO: maybe the cursor should be the job of the shell?
        const RECIVE_INPUT = 1 << 0;
        const DRAW_GRAPHICS = 1 << 1;
    }
}

#[allow(clippy::upper_case_acronyms)]
pub struct TTY<T: TTYInterface> {
    pub stdin_buffer: PageString,

    pub settings: TTYSettings,
    interface: T,
}

impl<T: TTYInterface> Write for TTY<T> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if self.settings.contains(TTYSettings::DRAW_GRAPHICS) {
            self.interface.write_str(s)?;
        }
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        if self.settings.contains(TTYSettings::DRAW_GRAPHICS) {
            self.interface.write_char(c)?;
        }
        Ok(())
    }
}

impl<T: TTYInterface> TTY<T> {
    pub fn new(interface: T) -> Self {
        Self {
            stdin_buffer: PageString::new(),
            interface,
            settings: TTYSettings::DRAW_GRAPHICS,
        }
    }

    pub fn clear(&mut self) {
        self.interface.clear();
        self.interface.set_cursor(0, 0);
    }

    pub fn enable_input(&mut self) {
        if !self.settings.contains(TTYSettings::RECIVE_INPUT) {
            self.settings |= TTYSettings::RECIVE_INPUT;
            _ = self.write_char('_');
        }
    }

    pub fn disable_input(&mut self) {
        if self.settings.contains(TTYSettings::RECIVE_INPUT) {
            self.settings &= !TTYSettings::RECIVE_INPUT;
            self.interface.backspace();
        }
    }

    pub fn peform_backspace(&mut self) {
        if !self.stdin_buffer.is_empty() {
            if self.settings.contains(TTYSettings::RECIVE_INPUT) {
                // removes the cursor `_`
                self.interface.backspace();
            }
            // backspace
            self.interface.backspace();
            self.stdin_buffer.pop();

            if self.settings.contains(TTYSettings::RECIVE_INPUT) {
                // puts the cursor `_`
                _ = self.write_char('_');
            }
        }
    }
}

lazy_static! {
    pub static ref FRAMEBUFFER_TERMINAL: RwLock<TTY<FrameBufferTTY<'static>>> =
        RwLock::new(TTY::new(FrameBufferTTY::new()));
}

impl<T: TTYInterface> HandleKey for TTY<T> {
    fn handle_key(&mut self, key: Key) {
        match key.code {
            KeyCode::PageDown => self.interface.scroll_down(),
            KeyCode::PageUp => self.interface.scroll_up(),
            KeyCode::KeyC if key.flags.contains(KeyFlags::CTRL | KeyFlags::SHIFT) => {
                self.clear();
                pspawn("Shell", "sys:/bin/Shell", &[], SpawnFlags::CLONE_RESOURCES).unwrap();
            }
            KeyCode::Backspace if self.settings.contains(TTYSettings::RECIVE_INPUT) => {
                self.peform_backspace();
            }
            _ => {
                if self.settings.contains(TTYSettings::RECIVE_INPUT) {
                    // remove the cursor `_`
                    self.interface.backspace();
                    let char = key.map_key();
                    if char != '\0' {
                        let _ = self.write_char(char);
                        self.stdin_buffer.push_char(char);
                    }
                    // put the cursor back
                    _ = self.write_char('_');
                }
            }
        }
    }
}

/// writes to the framebuffer terminal
#[doc(hidden)]
#[unsafe(no_mangle)]
pub fn _print(args: core::fmt::Arguments) {
    FRAMEBUFFER_TERMINAL.write().write_fmt(args).unwrap();
}
