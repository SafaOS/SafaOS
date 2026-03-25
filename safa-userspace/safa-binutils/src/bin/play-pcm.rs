use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    process::exit,
};

pub fn main() {
    let mut args = std::env::args();
    let name = args.next();
    let name = name.as_ref().map(|a| a.as_str()).unwrap_or("play-pcm");

    let Some(file) = args.next() else {
        eprintln!("{name}: Expected file to play");
        exit(-1);
    };

    let audio_devices = std::fs::read_dir("dev:/audio").expect("Failed to retrieve audio devices");
    let device_path = audio_devices
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().unwrap().is_file())
        .map(|e| e.path())
        .next()
        .expect("No audio device found");

    let mut read_from = File::open(file).expect("Failed to open PCM file");
    let mut write_to = OpenOptions::new()
        .write(true)
        .open(device_path)
        .expect("Failed to open audio device");

    let mut data = Vec::new();
    read_from
        .read_to_end(&mut data)
        .expect("Failed to read PCM File");

    let mut curr = &*data;
    while curr.len() != 0 {
        match write_to.write(curr) {
            Ok(n) => curr = &curr[n..],
            Err(e) => {
                eprintln!("Unexpected error: {e}");
                exit(-1);
            }
        }
    }
}
