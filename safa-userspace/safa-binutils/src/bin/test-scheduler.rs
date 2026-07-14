use std::{process::Command, time::Duration};

use safa_api::syscalls::misc::uptime;

#[inline(always)]
fn time() -> u64 {
    uptime()
}

const SLEEP_TIME: u64 = 1000;

pub fn test_scheduler(proc_num: u64, thread_per_proc: u64) {
    let sleep_time = SLEEP_TIME.to_string();
    let thread_per_proc = thread_per_proc.to_string();

    println!("Spawn {proc_num} processes test");

    let start_spawn_time = time();
    let mut children = Vec::with_capacity(proc_num as usize);
    for _ in 0..proc_num {
        let child = Command::new("sys:/bin/sched_test")
            .arg(&sleep_time)
            .arg(&thread_per_proc)
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

pub fn test_thread_spawning(thread_num: u64) {
    println!("Spawn {thread_num} threads test...");

    let start_spawn_time = time();
    let mut children = Vec::with_capacity(thread_num as usize);

    for thread in 0..thread_num {
        let handle = std::thread::spawn(move || {
            let curr = std::thread::current();
            let curr_id = curr.id();
            let curr_name = curr.name().unwrap_or("unnamed");
            println!(
                "Hello from: {thread} out of {thread_num}, curr_id is {curr_id:?}, curr_name is {curr_name}!"
            );
            std::thread::sleep(Duration::from_secs(1));
        });
        children.push(handle);
    }

    let end_spawn_time = time();

    let start_wait_time = time();
    for child in children {
        child.join().expect("couldn't wait for thread");
    }
    let end_wait_time = time();

    eprintln!("Spawn time: {}ms", end_spawn_time - start_spawn_time);
    eprintln!("Wait time: {}ms", end_wait_time - start_wait_time);
    eprintln!("Overall time: {}ms", end_wait_time - start_spawn_time);
}

pub fn main() {
    let mut args = std::env::args();
    let _name = args.next();
    let proc_num = args
        .next()
        .expect("no process number given")
        .parse::<u64>()
        .expect("invalid process number");

    test_scheduler(proc_num, 1);
    test_thread_spawning(proc_num);

    test_scheduler(proc_num / 3, 6);
}
