use clap::Parser;
use std::{io, path::PathBuf};

#[derive(Parser)]
/// Copy a file to a new path/another file
struct Args {
    /// the src file to copy
    src: PathBuf,
    /// the destination to copy to
    dest: PathBuf,
}

use extra::tri_io;
use safa_api::errors::SysResult;

fn copy(from: PathBuf, to: PathBuf) -> io::Result<u64> {
    let metadata = std::fs::metadata(&to);
    let src = from;

    let dest = match metadata {
        Ok(m) if m.file_type().is_dir() => {
            to.join(src.file_name().expect("no file name to copy from given"))
        }
        Err(_) | Ok(_) => to,
    };

    std::fs::copy(src, dest)
}

fn main() -> SysResult {
    let args = Args::parse();
    tri_io!(copy(args.src, args.dest));
    SysResult::Success
}
