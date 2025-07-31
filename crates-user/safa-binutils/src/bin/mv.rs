use clap::Parser;
use std::{io, path::PathBuf};

#[derive(Parser)]
/// Moves a file/directory from a path to another or moves a file to a directory
struct Args {
    /// the src file to move
    src: PathBuf,
    /// the destination to move to
    dest: PathBuf,
}

use extra::tri_io;
use safa_api::errors::SysResult;

fn move_paths(from: PathBuf, to: PathBuf) -> io::Result<()> {
    let dest_metadata = std::fs::metadata(&to);

    let src = from;

    let dest = match dest_metadata {
        Ok(m) if m.file_type().is_dir() => {
            to.join(src.file_name().expect("no file name to copy from given"))
        }
        Err(_) | Ok(_) => to,
    };

    std::fs::rename(src, dest)
}

fn main() -> SysResult {
    let args = Args::parse();
    tri_io!(move_paths(args.src, args.dest));
    SysResult::Success
}
