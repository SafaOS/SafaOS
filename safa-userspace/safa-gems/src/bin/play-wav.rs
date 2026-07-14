use std::{fs::File, path::PathBuf};

use libartemis::audio::AudioPlayer;
use safa_api::errors::{ErrorStatus, SysResult};

fn main() -> SysResult {
    let mut args = std::env::args();
    let program_name = args.next();
    let program_name = program_name
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("play-wav");

    let Some(file_path) = args.next() else {
        eprintln!("{program_name}: Expected WAV file path");
        return SysResult::err(ErrorStatus::NotEnoughArguments);
    };

    let path = PathBuf::from(file_path);
    let file = File::open(path).expect("Failed to open WAV file");
    let player = AudioPlayer::load_wav(file).expect("Failed to parse WAV file");

    player.play().expect("Failed to play WAV File");
    SysResult::ok(0)
}
