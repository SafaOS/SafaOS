use std::process::Command;

use safa_api::syscalls::uptime;

#[inline(always)]
fn time() -> u64 {
    uptime()
}

const SLEEP_TIME: u64 = 1000;

pub fn test_scheduler(proc_num: u64) {
    let start_spawn_time = time();
    let mut children = Vec::with_capacity(proc_num as usize);
    for _ in 0..proc_num {
        let child = Command::new("sys:/bin/sleep")
            .arg(SLEEP_TIME.to_string())
            .spawn()
            .expect("failed to spawn `sleep`");
        children.push(child);
    }
    let end_spawn_time = time();

    let start_wait_time = time();
    for (i, mut child) in children.into_iter().enumerate() {
        let output = child.wait();
        println!("child: {i} returned {output:?}\n");
    }
    let end_wait_time = time();

    println!("Spawn time: {}ms", end_spawn_time - start_spawn_time);
    println!("Wait time: {}ms", end_wait_time - start_wait_time);
    println!("Overall time: {}ms", end_wait_time - start_spawn_time);
}

pub fn main() {
    let mut args = std::env::args();
    let _name = args.next();
    let proc_num = args
        .next()
        .expect("no process number given")
        .parse::<u64>()
        .expect("invalid process number");

    println!(
        "starting with {proc_num} processes, at uptime: {}ms",
        uptime()
    );
    test_scheduler(proc_num);
}
