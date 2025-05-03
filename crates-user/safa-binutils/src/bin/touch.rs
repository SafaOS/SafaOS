use std::fs::OpenOptions;

use extra::tri_io;
use safa_api::errors::{ErrorStatus, SysResult};

fn main() -> SysResult {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        println!("touch: missing file path");
        return SysResult::Error(ErrorStatus::NotEnoughArguments);
    };

    tri_io!(OpenOptions::new().create_new(true).open(path));
    SysResult::Success
}
