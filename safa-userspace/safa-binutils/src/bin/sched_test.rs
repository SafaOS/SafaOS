use std::{
    thread,
    time::{Duration, Instant},
};

pub fn main() {
    let mut args = std::env::args();
    args.next();
    let amount = args.next().unwrap_or_else(|| {
        eprintln!("Usage: test <milliseconds> <threads>");
        std::process::exit(1);
    });

    let threads = args.next().unwrap_or_else(|| {
        eprintln!("Usage: test <milliseconds> <threads>");
        std::process::exit(1);
    });

    let wait_amount = amount.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("Invalid argument: {}", amount);
        std::process::exit(1);
    });

    let threads = threads.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("Invalid argument: {}", threads);
        std::process::exit(1);
    });

    let instant = Instant::now();

    for _ in 0..threads.saturating_sub(1) {
        thread::spawn(|| loop {});
    }

    while instant.elapsed() < Duration::from_millis(wait_amount) {
        core::hint::spin_loop();
    }
}
