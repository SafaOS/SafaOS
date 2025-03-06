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
    utils::{
        alloc::{PageBString, PageString},
        bstr::BStr,
    },
};

pub mod framebuffer;

/// defines the interface for a tty
/// a tty is a user-visible device that can be written to, and that user-input can be read from
/// it is recommened for the tty to support ansii escape sequences, some stuff will be managed by a
/// higher-level tty implementation `TTY` only writing to the tty is required
pub trait TTYInterface: Send + Sync {
    fn write_str(&mut self, s: &BStr);
    /// removes the character at the current cursor position
    /// and moves the cursor to the left
    fn backspace(&mut self);
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
        /// wether or not we are currently reciving input
        /// the cursor should work well if enabled correctly using `self.enable_input` and disabled
        /// using `self.disable_input`
        // TODO: maybe the cursor should be the job of the shell?
        const RECIVE_INPUT = 1 << 0;
        const DRAW_GRAPHICS = 1 << 1;
        const CANONICAL_MODE = 1 << 2;
        const ECHO_INPUT = 1 << 3;
    }
}

#[allow(clippy::upper_case_acronyms)]
pub struct TTY<T: TTYInterface> {
    /// stores the stdout buffer for write operations peformed on the tty device, allows to write to the tty at once instead of a peice by piece
    stdout_buffer: PageBString,
    /// stores the stdin buffer for read operations peformed on the tty device
    pub stdin_buffer: PageString,
    pub settings: TTYSettings,
    interface: T,
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
            stdin_buffer: PageString::new(),
            stdout_buffer: PageBString::with_capacity(4096),
            interface,
            settings: TTYSettings::DRAW_GRAPHICS
                | TTYSettings::CANONICAL_MODE
                | TTYSettings::ECHO_INPUT,
        }
    }

    pub fn clear(&mut self) {
        self.interface.clear();
        self.interface.set_cursor(0, 0);
    }

    pub fn enable_input(&mut self) {
        if !self.settings.contains(TTYSettings::RECIVE_INPUT) {
            self.settings |= TTYSettings::RECIVE_INPUT;
            self.interface.draw_cursor();
        }
    }

    pub fn disable_input(&mut self) {
        if self.settings.contains(TTYSettings::RECIVE_INPUT) {
            self.settings &= !TTYSettings::RECIVE_INPUT;
            self.interface.hide_cursor();
        }
    }

    pub fn peform_backspace(&mut self) {
        if !self.stdin_buffer.is_empty() {
            // backspace
            self.interface.backspace();
            self.stdin_buffer.pop();
        }
    }

    /// syncs the buffer by actually writing it to the interface
    pub fn sync(&mut self) {
        if self.settings.contains(TTYSettings::DRAW_GRAPHICS) {
            self.interface.write_str(self.stdout_buffer.as_bstr());
            self.stdout_buffer.clear();
        }
    }

    pub fn write_bstr(&mut self, s: &BStr) {
        self.stdout_buffer.push_bstr(s);
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
                self.interface.set_cursor(1, 1);
                pspawn("Shell", "sys:/safa", &[], SpawnFlags::CLONE_RESOURCES).unwrap();
            }
            KeyCode::Backspace
                if self.settings.contains(TTYSettings::RECIVE_INPUT)
                    && self.settings.contains(TTYSettings::CANONICAL_MODE) =>
            {
                self.interface.hide_cursor();
                self.peform_backspace();
                self.interface.draw_cursor();
            }
            _ if self.settings.contains(TTYSettings::RECIVE_INPUT) => {
                let mapped = key.map_key();
                if mapped.is_empty() {
                    return;
                }
                self.stdin_buffer.push_str(mapped);

                if self.settings.contains(TTYSettings::ECHO_INPUT) {
                    self.interface.hide_cursor();
                    let _ = self.interface.write_str(mapped.into());
                    self.interface.draw_cursor();
                }
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
