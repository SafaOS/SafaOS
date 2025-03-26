use extra::tri_io;
use safa_api::errors::{ErrorStatus, SysResult};

fn main() -> SysResult {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        println!("mkdir: missing directory path");
        return SysResult::Error(ErrorStatus::NotEnoughArguments);
    };

    tri_io!(std::fs::create_dir(path));
    SysResult::Success
}
