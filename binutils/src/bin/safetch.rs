use extra::{
    json::{CpuInfo, KernelInfo, MemInfo},
    tri_io,
};
use owo_colors::OwoColorize;
use safa_api::errors::SysResult;
use std::{
    fs::File,
    io::{self, BufReader, Write, stdout},
};

macro_rules! print_item {
    ($($arg:tt)*) => {
        println!("\x1b[31C{}", format_args!($($arg)*))
    };
}

macro_rules! print_field {
    ($name: literal, $($arg:tt)*) => {
        print_item!("{}: {}", ($name).bright_magenta(), format_args!($($arg)*))
    };
}

fn print_logo() -> io::Result<()> {
    let mut stdout = stdout();
    let logo_file = File::open("sys:/logo.txt")?;
    let mut logo_reader = BufReader::new(logo_file);

    io::copy(&mut logo_reader, &mut stdout)?;
    stdout.flush()?;
    Ok(())
}

fn print_colors() {
    print!(
        "\x1b[31C\x1b[30m\x1b[40m   \x1b[31m\x1b[41m   \x1b[32m\x1b[42m   \x1b[33m\x1b[43m   \x1b[34m\x1b[44m   \x1b[35m\x1b[45m   \x1b[36m\x1b[46m   \x1b[37m\x1b[47m   \x1b[m\n"
    );
    print!(
        "\x1b[31C\x1b[90m\x1b[100m   \x1b[91m\x1b[101m   \x1b[92m\x1b[102m   \x1b[93m\x1b[103m   \x1b[94m\x1b[104m   \x1b[95m\x1b[105m   \x1b[96m\x1b[106m   \x1b[97m\x1b[107m   \x1b[m"
    );
}

fn print() -> io::Result<()> {
    let meminfo = MemInfo::fetch()?;
    let cpuinfo = CpuInfo::fetch()?;
    let kernelinfo = KernelInfo::fetch()?;
    print_logo()?;

    print!("\x1b[11A");

    print_item!(
        "{}@{}",
        "root".bright_magenta(),
        "localhost".bright_magenta()
    );

    print_field!("OS", "SafaOS");
    print_field!(
        "Kernel",
        "{} (v{} built on {})",
        kernelinfo.name(),
        kernelinfo.version(),
        kernelinfo.compile_date()
    );

    match kernelinfo.uptime_seconds() {
        60..3600 => {
            let (minutes, seconds) = kernelinfo.uptime_minutes();
            print_field!("Uptime", "{minutes}m{seconds}s")
        }

        3600.. => {
            let (hours, minutes, seconds) = kernelinfo.uptime_hours();
            print_field!("Uptime", "{hours}h{minutes}m{seconds}s")
        }

        seconds => print_field!("Uptime", "{seconds}s"),
    }

    print_field!("Terminal", "dev:/tty");
    print_field!("CPU", "{}", cpuinfo.model());
    print_field!(
        "Memory",
        "{}MiB/{}MiB",
        meminfo.used_mib(),
        meminfo.total_mib()
    );

    print!("\x1b[1B");
    print_colors();

    println!("\x1b[2B");
    Ok(())
}

fn main() -> SysResult {
    tri_io!(print());
    SysResult::Success
}
