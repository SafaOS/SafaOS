use std::fmt::Display;

use extra::{ostring_to_string, tri_io};
use owo_colors::OwoColorize;
use safa_api::errors::SysResult;

fn main() -> SysResult {
    let args = std::env::args();
    let mut raw = false;

    for arg in args {
        match arg.as_str() {
            "--raw" | "-r" => raw = true,
            "--color" | "-c" => raw = false,
            _ => {}
        }
    }

    let current_dir = tri_io!(std::fs::read_dir("."));
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut other = Vec::new();

    for file in current_dir {
        let file = tri_io!(file);
        let ty = file.file_type().unwrap();
        let name = ostring_to_string(file.file_name());

        if ty.is_dir() {
            directories.push(name);
        } else if ty.is_file() {
            files.push(name);
        } else {
            // TODO: count devices as others somehow? for now is_file returns true on devices
            other.push(name);
        }
    }

    // directories first
    for dir in directories {
        let display: &dyn Display = if !raw { &dir.blue() } else { &dir };
        println!("{}", display);
    }

    for other in other {
        let display: &dyn Display = if !raw { &other.red() } else { &other };
        println!("{}", display);
    }

    for f in files {
        println!("{}", f);
    }

    SysResult::Success
}
