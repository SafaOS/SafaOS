use std::path::PathBuf;

use libgem::image::QOIImage;
use libopal::window::{Window, WindowFlags};

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

            let mut wall_window = Window::create(WindowFlags::BG_WINDOW, width, height);
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

fn main() {
    let _ = init_wallpaper();
    loop {}
}
