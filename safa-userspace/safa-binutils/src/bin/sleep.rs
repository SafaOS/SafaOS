use std::time::Duration;

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

    std::thread::sleep(Duration::from_millis(amount));
}
