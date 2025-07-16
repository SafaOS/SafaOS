use std::path::PathBuf;

use clap::Parser;
use extra::tri_io;
use safa_api::errors::SysResult;

#[derive(Parser)]
/// Removes a file from a given assigned path
struct Args {
    /// The target path to remove
    path: PathBuf,
    /// Indicates that the target path is a directory
    #[arg(short, long, default_value_t = false)]
    dir: bool,
    /// Indicates the the target path, if a directory should be removed recursively
    #[arg(short, long, default_value_t = false)]
    recursive: bool,
}

fn remove(path: PathBuf, directory: bool, recursive: bool) -> std::io::Result<()> {
    match (directory, recursive) {
        (true, true) => std::fs::remove_dir_all(path),
        (false, false) => std::fs::remove_file(path),

        (true, false) => std::fs::remove_dir(path),
        (false, true) => panic!("cannot recursively remove in file mode"),
    }
}

fn main() -> SysResult {
    let args = Args::parse();
    let results = remove(args.path, args.dir, args.recursive);
    tri_io!(results);
    SysResult::Success
}
