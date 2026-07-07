mod display;
mod utils;

use std::{
    io::{Cursor, Read, Write},
    os::safaos::io::IoUtils,
    process::Command,
    str,
    sync::{LazyLock, Mutex},
    time::Instant,
};

use libartemis::audio::AudioPlayer;
use libgems::{
    Color, Data, Padding, WindowBuilder,
    shards::{Shard, ShardsExt},
};
use libopal::{
    WindowEvent,
    defs::KeyModifiers,
    event::{KeyCode, KeyEventKind},
};
use safa_api::abi::poll::{PollEntry, PollEvents};

use crate::display::{TermDisplay, TermRequest};
use std::os::safaos::AsRawResource;

pub const FONT_HEIGHT: f32 = 12.;
pub const LINE_HEIGHT: f32 = 14.;
pub const FONT_WIDTH: f32 = 7.;
const WIDTH: u32 = 640;
const HEIGHT: u32 = 560;

const CHAR_WIDTH: u32 = (WIDTH / FONT_WIDTH as u32) - 2 /* padding */;
const CHAR_LINES: u32 = 150;

const TITLE: &str = "Terminal";
const BG_COLOR: Color = Color::rgb(0x28, 0x28, 0x28).with_alpha(0xF0);

static ICON: &[u8] = include_bytes!("../../../assets/terminal.bmp");

use libopal::keys::keycode_to_char;
struct TerminalData {
    statemachine: vte::Parser,
    buf: Vec<u8>,
    requests: Vec<TermRequest>,
}
enum Message {
    NewData,
    ScrollView(i32),
}

fn build_ui() -> impl Shard<TerminalData, Message> + 'static {
    TermDisplay::new(CHAR_WIDTH, CHAR_LINES)
        .on_msg(
            |_, data: &mut Data<TerminalData, Message>, msg, this| match msg {
                Message::NewData => {
                    let data = &mut **data;

                    let instant = Instant::now();
                    data.statemachine.advance(this, &data.buf);
                    data.requests.extend(this.collect_requests());

                    let elapsed = instant.elapsed();
                    println!(
                        "elapsed: {}ms, parsing {}",
                        elapsed.as_millis(),
                        data.buf.len()
                    );
                }
                Message::ScrollView(amoun) => this.move_view_by(*amoun),
            },
        )
        .fix_size(WIDTH as f32 - (FONT_WIDTH * 2.) as f32, HEIGHT as f32)
        .pad(Padding::lr(FONT_WIDTH as f32))
}

const BEEP_DATA: &[u8] = include_bytes!("../../../assets/beep.wav");
static BEEP_AUDIO: LazyLock<AudioPlayer<Cursor<&'static [u8]>>> = LazyLock::new(|| {
    AudioPlayer::load_wav(Cursor::new(BEEP_DATA)).expect("Failed to load beep audio")
});

static BEEP_MUT: Mutex<()> = Mutex::new(());
static BEEP_THREAD: std::sync::Condvar = std::sync::Condvar::new();

fn beep_main() {
    loop {
        let _guard = BEEP_THREAD
            .wait(BEEP_MUT.lock().expect("Failed to lock mutex"))
            .expect("Failed to lock mutex");
        BEEP_AUDIO.play().expect("Failed to play beep audio");
        BEEP_AUDIO.reset();
    }
}

pub fn play_beep() {
    BEEP_THREAD.notify_one();
}

fn main() {
    std::thread::spawn(|| beep_main());

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

    let window = WindowBuilder::new(WIDTH, HEIGHT)
        .background(BG_COLOR)
        .icon(ICON)
        .title(TITLE)
        .build(build_ui());
    let mut app = libgems::App::new(TerminalData {
        statemachine: vte::Parser::new(),
        buf: Vec::with_capacity(4096),
        requests: Vec::new(),
    })
    .window(window);

    let mut write_to = mother
        .try_clone()
        .expect("Cloning mother should never fail");

    let mut read_from = mother;

    let mut poll_entries = [
        PollEntry::new(0, PollEvents::NONE),
        PollEntry::new(
            read_from.as_raw_resource() as u32,
            PollEvents::DATA_AVAILABLE,
        ),
    ];

    loop {
        let events = app.try_handle_events_with_poll(&mut poll_entries);

        let len = read_from
            .read_to_end(&mut app.data_mut().buf)
            .expect("Failed to read stdout");
        if len != 0 {
            app.broadcast_message(Message::NewData);
            app.data_mut().buf.clear();
        }

        for request in app.data_mut().requests.drain(..) {
            match request {
                TermRequest::CursorRequest((x, y)) => {
                    eprintln!("Cursor requested at: x:{x}, y:{y}");
                    write!(write_to, "\x1b[{};{}R", y + 1, x + 1)
                        .expect("Failed to write to stdio");
                }
            }
        }

        let mut write = |b: &[u8]| write_to.write(b).expect("Failed to write to stdin");
        let mut scroll_lines = 0;
        if let Some(events) = events {
            for event in events.iter().map(|w_eve| w_eve.event()) {
                match event {
                    WindowEvent::Key(k_eve) => {
                        if k_eve.kind == KeyEventKind::Press {
                            if let Some((normi_c, shifted_c, capslock_c)) =
                                keycode_to_char(k_eve.code)
                            {
                                let c = if k_eve.modifiers.contains(KeyModifiers::SHIFT) {
                                    shifted_c
                                } else if k_eve.modifiers.contains(KeyModifiers::CAPSLOCK) {
                                    capslock_c
                                } else {
                                    normi_c
                                };

                                let mut tmp = [0u8; 4];
                                let s = c.encode_utf8(&mut tmp);

                                write(s.as_bytes());
                            } else {
                                match k_eve.code {
                                    KeyCode::Down => {
                                        write(b"\x1b[B");
                                    }
                                    KeyCode::Up => {
                                        write(b"\x1b[A");
                                    }
                                    KeyCode::Right
                                        if k_eve
                                            .modifiers
                                            .contains(KeyModifiers::CTRL | KeyModifiers::SHIFT) =>
                                    {
                                        write(b"\x1b[A");
                                    }
                                    KeyCode::Left
                                        if k_eve
                                            .modifiers
                                            .contains(KeyModifiers::CTRL | KeyModifiers::SHIFT) =>
                                    {
                                        write(b"\x1b[B");
                                    }
                                    KeyCode::Left => {
                                        write(b"\x1b[D");
                                    }
                                    KeyCode::Right => {
                                        write(b"\x1b[C");
                                    }
                                    KeyCode::Tab
                                        if k_eve.modifiers.contains(KeyModifiers::SHIFT) =>
                                    {
                                        write(b"\x1b[Z");
                                    }
                                    KeyCode::Tab => {
                                        write(b"\t");
                                    }
                                    KeyCode::Home => {
                                        write(b"\x1b[1~");
                                    }
                                    KeyCode::Delete => {
                                        write(b"\x1b[3~");
                                    }
                                    KeyCode::End => {
                                        write(b"\x1b[4~");
                                    }
                                    KeyCode::Backspace => {
                                        write(&[ERASE_CHAR]);
                                    }
                                    KeyCode::Return => {
                                        write(b"\n");
                                    }
                                    KeyCode::PageUp
                                        if k_eve.modifiers.contains(KeyModifiers::SHIFT) =>
                                    {
                                        scroll_lines -= 3;
                                    }

                                    KeyCode::PageDown
                                        if k_eve.modifiers.contains(KeyModifiers::SHIFT) =>
                                    {
                                        scroll_lines += 3;
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

        if scroll_lines != 0 {
            app.broadcast_message(Message::ScrollView(scroll_lines));
        }
    }
}
