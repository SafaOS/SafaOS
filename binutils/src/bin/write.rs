use std::fs;

use extra::tri_io;
use safa_api::errors::{ErrorStatus, SysResult};

fn main() -> SysResult {
    let mut args = std::env::args().skip(1);
    let Some(file) = args.next() else {
        println!("write: missing file path");
        return SysResult::Error(ErrorStatus::NotEnoughArguments);
    };

    let Some(data) = args.next() else {
        println!("write: missing data to write to file");
        return SysResult::Error(ErrorStatus::NotEnoughArguments);
    };

    tri_io!(fs::write(file, data));
    SysResult::Success
}
