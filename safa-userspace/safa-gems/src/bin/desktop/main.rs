use std::path::Path;

use image::imageops;
use libgems::{AppEnv, Color, WindowBuilder};
use libopal::{
    WindowEvent,
    window::{Window, WindowFlags},
};

use crate::main_dock::{DockData, DockMessage};

mod main_dock;
mod task_button;

/// Slow without SMP, prints some events
const REALLY_VERBOSE: bool = false;

fn init_wallpaper(wallpath: Option<&Path>) -> Option<Window> {
    let Some(path) = wallpath else {
        return None;
    };

    let now = std::time::Instant::now();

    let Ok(image) = image::open(path) else {
        eprintln!("Failed to open wallpaper image");
        return None;
    };
    let elapsed = now.elapsed();
    println!("Decoding took {}ms", elapsed.as_millis());

    let (width, height) = libopal::get_screen_dimensions();
    let now = std::time::Instant::now();
    let scaled = image
        .resize_to_fill(width, height, imageops::FilterType::CatmullRom)
        .into_rgba8();
    let elapsed = now.elapsed();
    println!("Scaling took {}ms", elapsed.as_millis());

    let mut wall_window = Window::create(
        "",
        WindowFlags::BG_WINDOW | WindowFlags::NO_DECORATIONS,
        width,
        height,
        None,
        None,
    );

    for (src, dst) in scaled.pixels().zip(wall_window.pixels_mut().iter_mut()) {
        *dst = libgems::Color::rgba(src.0[0], src.0[1], src.0[2], src.0[3]);
    }
    wall_window.redraw(0, 0, width, height);
    Some(wall_window)
}

fn main() {
    let env = AppEnv::sys_theme();
    let _win = init_wallpaper(
        uopal_desktop::themes::ThemesDatabase::try_load()
            .expect("Failed to load themes")
            .sys_theme()
            .background_path
            .as_ref()
            .map(|p| p.as_path()),
    );

    let data = DockData::new();

    let (screen_width, screen_height) = libopal::get_screen_dimensions();

    let height = 64;
    let window = WindowBuilder::new(screen_width, height)
        .y(Some((screen_height - (40 * 2) - 16) as i32))
        .flags(WindowFlags::OVERLAY_WINDOW | WindowFlags::NO_DECORATIONS)
        .title("")
        .background(Color::NONE)
        .build(main_dock::build_ui(&env, screen_width, height));
    let mut app = libgems::App::new(data).with_env(env);
    let win_id = app.add_window(window);

    loop {
        // app.redraw_needed();
        let events = app.wait_for_events();
        if REALLY_VERBOSE {
            println!("taskbar events: {events:?}");
        }

        for win_even in (&*events)
            .iter()
            .filter(|win_eve| win_eve.receiver() == win_id)
        {
            let event = win_even.event();
            match event {
                WindowEvent::GlobalWindowAttached(win) => {
                    app.broadcast_message(DockMessage::Attached(win.win_id()));
                }
                WindowEvent::GlobalWindowDeatached(win) => {
                    app.broadcast_message(DockMessage::Deatached(win.win_id()))
                }
                WindowEvent::GlobalWindowFocusChanged(change, window) => {
                    app.broadcast_message(DockMessage::FocusChanged(window, change.is_focused()));
                }
                _ => {}
            }
        }
    }
}
