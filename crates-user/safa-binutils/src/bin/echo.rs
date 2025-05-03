fn main() {
    let args = std::env::args();
    let args = args.skip(1);

    for arg in args {
        print!("{}", arg);
    }

    println!();
}
