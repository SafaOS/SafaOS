use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write, stdout},
};

use clap::Parser;
use safa_api::errors::SysResult;

macro_rules! tri_io {
    ($expr: expr) => {
        tri!($expr.map_err(|e| safa_api::errors::err_from_io_error_kind(e.kind())))
    };
}

macro_rules! tri {
    ($expr: expr) => {
        match $expr {
            Ok(data) => data,
            Err(e) => return safa_api::errors::SysResult::Error(e),
        }
    };
}

#[derive(Parser)]
/// Your only tool for viewing the logs of SafaOS's kernel.
struct Arguments {
    #[arg(short, long)]
    /// The amount of lines to read from the end of the logs, by default all lines are read.
    end: Option<u32>,
}

fn majala() -> io::Result<()> {
    let args = Arguments::parse();
    let journal = File::open("rod:/eve-journal")?;

    let mut stdout = stdout();
    let mut reader = BufReader::new(journal);

    let full_read = args.end.is_none();

    if full_read {
        io::copy(&mut reader, &mut stdout)?;
        println!();
    }

    if let Some(from_end) = args.end {
        let mut lines = Vec::new();
        // not the most efficient way to do this, but it works
        for line in reader.lines() {
            lines.push(line?);
        }

        let mut to_print = Vec::with_capacity(from_end as usize);

        for _ in 0..from_end {
            if let Some(line) = lines.pop() {
                to_print.push(line);
            }
        }

        for line in to_print.into_iter().rev() {
            stdout.write_all(line.as_bytes())?;
            stdout.write_all(b"\n")?;
        }
    }

    Ok(())
}
fn main() -> SysResult {
    tri_io!(majala());
    SysResult::Success
}
