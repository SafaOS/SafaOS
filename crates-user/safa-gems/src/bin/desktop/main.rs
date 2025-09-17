use std::path::PathBuf;

use libgem::{
    App, Gem, GemConfig,
    canvas::Pixel,
    element::container::{ContainerLayout, ContainerStyles, VerticalLayout},
    image::QOIImage,
};
use libopal::{
    Event,
    window::{Window, WindowFlags},
};

use crate::main_dock::MainDock;

mod main_dock;
mod task_button;

const WALLPAPERS_DIR: &str = "sys:/usr/pictures/wallpapers";

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

            let mut wall_window =
                Window::create("", WindowFlags::BG_WINDOW, width, height, None, None);
            wall_window
                .pixels_mut()
                .copy_from_slice(unsafe { std::mem::transmute(scaled.get_pixels()) });
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

struct TaskBar;
impl Gem for TaskBar {}
impl TaskBar {
    pub fn init() -> App<Self> {
        let (screen_width, screen_height) = libopal::get_screen_dimensions();

        let config = GemConfig::new("Taskbar", screen_width, 48)
            .with_border(None)
            .with_position(0, (screen_height - (40 * 2) - 16) as i32)
            .with_win_flags(WindowFlags::OVERLAY_WINDOW)
            .with_bg_color(Pixel::NONE);
        Self.init(config)
    }
}

fn main() {
    let _ = init_wallpaper();
    let mut taskbar = TaskBar::init();
    taskbar.body().set_styles(
        ContainerStyles::new().with_layout(ContainerLayout::Vertical(
            VerticalLayout::new().with_align_center(true),
        )),
    );

    let main_dock = MainDock::new();
    let main_dock_id = taskbar.add_element(main_dock);
    let win_id = taskbar.win().id();

    loop {
        taskbar.redraw();
        let events = taskbar.handle_events_blocking();
        let main_dock: &mut MainDock = unsafe {
            taskbar
                .body()
                .get_element_as_mut(main_dock_id)
                .unwrap_unchecked()
        };
        println!("taskbar events: {events:?}");
        for win_even in (&*events).iter().filter(|win_eve| win_eve.win() == win_id) {
            let event = win_even.event();
            match event {
                Event::GlobalWindowAttached(win) => main_dock.attached(win.win_id()),
                Event::GlobalWindowDeatached(win) => main_dock.deatached(win.win_id()),
                Event::GlobalWindowFocused(eve) => main_dock.focus_changed(eve.win_id(), true),
                Event::GlobalWindowUnfocused(eve) => main_dock.focus_changed(eve.win_id(), false),
                _ => {}
            }
        }
    }
}
