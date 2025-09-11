mod term_display;

use std::{
    io::{Read, Write},
    os::safaos::io::IoUtils,
    process::Command,
    str,
    time::Instant,
};

use libgem::{
    App, Gem, GemConfig,
    libopal::{
        Event,
        event::{KeyCode, KeyEventKind},
        window::Pixel,
    },
};

use crate::term_display::TerminalElement;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 560;
const TITLE: &str = "Terminal";
const BG_COLOR: Pixel = Pixel::from_rgb(0x28, 0x28, 0x28).with_alpha(0xF0);

const fn keycode_to_char(code: KeyCode) -> Option<char> {
    match code {
        // letters
        KeyCode::KeyA => Some('a'),
        KeyCode::KeyB => Some('b'),
        KeyCode::KeyC => Some('c'),
        KeyCode::KeyD => Some('d'),
        KeyCode::KeyE => Some('e'),
        KeyCode::KeyF => Some('f'),
        KeyCode::KeyG => Some('g'),
        KeyCode::KeyH => Some('h'),
        KeyCode::KeyI => Some('i'),
        KeyCode::KeyJ => Some('j'),
        KeyCode::KeyK => Some('k'),
        KeyCode::KeyL => Some('l'),
        KeyCode::KeyM => Some('m'),
        KeyCode::KeyN => Some('n'),
        KeyCode::KeyO => Some('o'),
        KeyCode::KeyP => Some('p'),
        KeyCode::KeyQ => Some('q'),
        KeyCode::KeyR => Some('r'),
        KeyCode::KeyS => Some('s'),
        KeyCode::KeyT => Some('t'),
        KeyCode::KeyU => Some('u'),
        KeyCode::KeyV => Some('v'),
        KeyCode::KeyW => Some('w'),
        KeyCode::KeyX => Some('x'),
        KeyCode::KeyY => Some('y'),
        KeyCode::KeyZ => Some('z'),

        // digits
        KeyCode::Key0 => Some('0'),
        KeyCode::Key1 => Some('1'),
        KeyCode::Key2 => Some('2'),
        KeyCode::Key3 => Some('3'),
        KeyCode::Key4 => Some('4'),
        KeyCode::Key5 => Some('5'),
        KeyCode::Key6 => Some('6'),
        KeyCode::Key7 => Some('7'),
        KeyCode::Key8 => Some('8'),
        KeyCode::Key9 => Some('9'),

        KeyCode::Space => Some(' '),
        KeyCode::Comma => Some(','),
        KeyCode::Dot => Some('.'),
        KeyCode::Slash => Some('/'),
        KeyCode::Semicolon => Some(';'),
        KeyCode::BackQuote => Some('`'),
        KeyCode::LeftBrace => Some('['),
        KeyCode::RightBrace => Some(']'),
        KeyCode::BackSlash => Some('\\'),
        KeyCode::Minus => Some('-'),
        KeyCode::Equals => Some('='),
        KeyCode::DoubleQuote => Some('"'),

        _ => None,
    }
}

struct Terminal;
impl Gem for Terminal {}

impl Terminal {
    fn init() -> App<Self> {
        Self.init(GemConfig::new(TITLE, WIDTH, HEIGHT).with_bg_color(BG_COLOR))
    }
}

fn main() {
    const SET_FLAGS: u16 = 1;
    const ECHO: u64 = 1 << 0;
    const CANONICAL: u64 = 1 << 1;
    const ECHO_ERASE: u64 = 1 << 2;
    const ERASE_CHAR: u8 = 0x7f;

    let shell = std::env::var("SHELL").expect("Failed to get SHELL");
    println!("Using the shell: {shell}");

    let (mother, child) = std::os::safaos::io::create_vtty().expect("Failed to create VTTY");
    mother
        .send_command(SET_FLAGS, ECHO | CANONICAL | ECHO_ERASE)
        .expect("Failed to setup VTTY");

    Command::new(shell)
        .arg("-i")
        .stdin(child.try_clone().unwrap())
        .stderr(child.try_clone().unwrap())
        .stdout(child.try_clone().unwrap())
        .spawn()
        .expect("Failed to spawn shell");

    let mut term = Terminal::init();
    let console_editor = TerminalElement::new(WIDTH, HEIGHT);
    let id = term.add_element(console_editor);

    let mut buf = Vec::with_capacity(4096);
    let mut write_to = mother
        .try_clone()
        .expect("Cloning mother should never fail");
    let mut write = move |b: &[u8]| write_to.write(b).expect("Failed to write to stdin");
    let mut read_from = mother;

    let mut statemechaine = vte::Parser::new();

    loop {
        term.redraw();

        let events = term.try_handle_events();
        let console: &mut TerminalElement = term.body().get_element_as_mut(id).expect("SDASsada??");

        let len = read_from
            .read_to_end(&mut buf)
            .expect("Failed to read stdout");
        if len != 0 {
            let instant = Instant::now();
            statemechaine.advance(console, &buf);
            let elapsed = instant.elapsed();
            println!("elapsed: {}ms, parsing {}", elapsed.as_millis(), buf.len());
            buf.clear();
        }

        if let Some(events) = events {
            for event in &*events {
                match event {
                    Event::Key(k_eve) => {
                        if k_eve.kind == KeyEventKind::Press {
                            if let Some(c) = keycode_to_char(k_eve.code) {
                                let mut tmp = [0u8; 4];
                                let s = c.encode_utf8(&mut tmp);

                                write(s.as_bytes());
                            } else {
                                match k_eve.code {
                                    KeyCode::Left => {
                                        write(b"\x1b[D");
                                    }
                                    KeyCode::Right => {
                                        write(b"\x1b[A");
                                    }
                                    KeyCode::Backspace => {
                                        write(&[ERASE_CHAR]);
                                    }
                                    KeyCode::Return => {
                                        write(b"\n");
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
