use extra::tri_io;
use safa_api::errors::{ErrorStatus, SysResult};
use std::{
    fs::File,
    io::{self, BufReader, Write, stdin, stdout},
};

use SysResult::*;

fn cat_file(path: &str) -> io::Result<()> {
    let mut stdout = stdout();
    let opened = File::open(path)?;
    let mut reader = BufReader::new(opened);

    io::copy(&mut reader, &mut stdout)?;
    println!();
    Ok(())
}
fn main() -> SysResult {
    let mut args = std::env::args().skip(1);
    if args.len() > 1 {
        println!("cat: too much arguments");
        return Error(ErrorStatus::Generic);
    }

    match args.next() {
        Some(path) => tri_io!(cat_file(&path)),
        None => {
            let mut buffer = String::new();
            let stdin = stdin();
            let mut stdout = stdout();
            loop {
                stdin
                    .read_line(&mut buffer)
                    .expect("failed to read from stdin");

                print!("{}", buffer);
                stdout.flush().expect("failed to flush stdout");

                buffer.clear();
            }
        }
    }

    Success
}
