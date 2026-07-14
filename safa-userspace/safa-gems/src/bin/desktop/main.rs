use std::{path::Path, time::Duration};

use image::imageops;
use libgems::{
    AppEnv, Color, WindowBuilder,
    shards::{Label, Shard, ShardsExt, Stack},
};
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

fn build_top_dock_ui() -> impl Shard<DockData, DockMessage> {
    Stack::column()
        .justify(libgems::shards::Justify::SpaceAround)
        .align(libgems::shards::AxisAlign::Center)
        .with(Label::from_str("").on_update(|_data, this| {
            this.set_text(chrono::Utc::now().format("%Y %b %d, %H:%M UTC").to_string());
        }))
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

    let dock_window = WindowBuilder::new(screen_width, height)
        .y(Some((screen_height - (40 * 2) - 16) as i32))
        .flags(WindowFlags::OVERLAY_WINDOW | WindowFlags::NO_DECORATIONS)
        .title("")
        .background(Color::NONE)
        .build(main_dock::build_ui(&env, screen_width, height));

    let top_dock_window = WindowBuilder::new(screen_width, 24)
        .flags(WindowFlags::OVERLAY_WINDOW | WindowFlags::NO_DECORATIONS)
        .title("")
        .y(Some(0))
        .build(
            build_top_dock_ui()
                .fix_width(screen_width as f32)
                .fix_height(24.),
        );

    let mut app = libgems::App::new(data)
        .with_env(env)
        .window(top_dock_window);
    let d_win_id = app.add_window(dock_window);

    loop {
        let Some(events) = app.try_wait_for_events_timeout(Some(Duration::from_secs(1))) else {
            app.update();
            continue;
        };
        if REALLY_VERBOSE {
            println!("taskbar events: {events:?}");
        }

        for win_even in (&*events)
            .iter()
            .filter(|win_eve| win_eve.receiver() == d_win_id)
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
