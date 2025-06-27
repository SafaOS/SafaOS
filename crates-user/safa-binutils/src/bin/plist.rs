use std::fmt::Display;

use extra::{json::ProcessInfo, tri_io};
use owo_colors::OwoColorize;
use safa_api::errors::SysResult;

fn main() -> SysResult {
    let (processes, name_alignment, pid_alignment) = tri_io!(ProcessInfo::fetch_all());
    let pid_alignment = pid_alignment.max("ppid".len());

    let write = |name: &dyn Display,
                 pid: &dyn Display,
                 ppid: &dyn Display,
                 is_alive: &dyn Display| {
        println!(
            "{name:name_alignment$}:    {pid:<pid_alignment$}     {ppid:<pid_alignment$}     {is_alive:8}",
        );
    };

    write(
        &"name".blue(),
        &"pid".bright_red(),
        &"ppid".bright_red(),
        &"is_alive".bright_yellow(),
    );

    println!(
        "{:-<width$}",
        "",
        width = pid_alignment * 2 + name_alignment + 3 * 5 + 8
    );

    for p in processes {
        let is_alive: &dyn Display = if p.is_alive() {
            &"true".bright_green()
        } else {
            &"false".bright_red()
        };

        write(
            &p.name().blue(),
            &p.pid().bright_red(),
            &p.ppid().bright_red(),
            is_alive,
        );
    }
    SysResult::Success
}
