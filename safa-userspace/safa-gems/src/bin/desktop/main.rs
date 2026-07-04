use std::path::PathBuf;

use libgem::{canvas::Pixel, image::QOIImage};
use libgems::{AppEnv, WindowBuilder};
use libopal::{
    WindowEvent,
    window::{Window, WindowFlags},
};

use crate::main_dock::{DockData, DockMessage};

mod main_dock;
mod task_button;

const WALLPAPERS_DIR: &str = "sys:/usr/pictures/wallpapers";
/// Slow without SMP, prints some events
const REALLY_VERBOSE: bool = false;

fn get_wallpapers() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(WALLPAPERS_DIR) else {
        return Vec::new();
    };
    dir.filter_map(|entry| entry.ok())
        .filter(|ent| ent.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.path())
        .collect()
}

fn init_wallpaper() -> Option<Window> {
    let wallpapers = get_wallpapers();
    let chosen_wall_path = wallpapers.get(0)?;

    match chosen_wall_path.extension() {
        Some(ext) if ext.as_encoded_bytes() == b"qoi" => {
            let now = std::time::Instant::now();
            let data = std::fs::read(chosen_wall_path).expect("Failed to read wallpaper");
            let decoded = match QOIImage::decode(&data) {
                Ok(decoded) => decoded,
                Err(err) => {
                    println!("Failed to decode QOI wallpaper err: {:?}", err);
                    return None;
                }
            };
            let elapsed = now.elapsed();
            println!("Decoding took {}ms", elapsed.as_millis());

            let (width, height) = libopal::get_screen_dimensions();
            let now = std::time::Instant::now();
            let scaled = decoded.into_scaled_image(width, height, libgem::image::ScaleType::Catrom);
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

            let pixels = scaled.get_pixels();
            wall_window.pixels_mut()[..pixels.len()]
                .copy_from_slice(unsafe { core::mem::transmute(pixels) });
            wall_window.redraw(0, 0, width, height);
            Some(wall_window)
        }
        Some(ext) => {
            println!("Unsupported extension: {}", ext.display());
            return None;
        }
        None => {
            println!("Wallpaper has no extension");
            return None;
        }
    }
}

fn main() {
    let _win = init_wallpaper();

    let data = DockData::new();

    let (screen_width, screen_height) = libopal::get_screen_dimensions();

    let height = 64;
    let env = AppEnv::sys_theme();
    let window = WindowBuilder::new(screen_width, height)
        .y(Some((screen_height - (40 * 2) - 16) as i32))
        .flags(WindowFlags::OVERLAY_WINDOW | WindowFlags::NO_DECORATIONS)
        .title("")
        .background(Pixel::NONE)
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
