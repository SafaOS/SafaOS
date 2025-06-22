use safa_api::syscalls::uptime;

pub fn main() {
    let mut args = std::env::args();
    args.next();
    let amount = args.next().unwrap_or_else(|| {
        eprintln!("Usage: sleep <milliseconds>");
        std::process::exit(1);
    });

    let amount = amount.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("Invalid argument: {}", amount);
        std::process::exit(1);
    });

    let start_time = uptime();
    let end_time = start_time + amount;
    while uptime() < end_time {
        core::hint::spin_loop();
    }
}
