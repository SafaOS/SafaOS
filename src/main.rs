use std::process::Command;

use clap::Parser;
use cli::{BuildOpts, Cli, SubCommand};
use safa_builder::ROOT_REPO_PATH;

mod cli;

fn main() {
    let program = std::env::args().next().unwrap();
    let args = Cli::parse();

    let (tests, build_args, run_opts) = match args.command {
        None => (false, args.run_args.build_args, Some(args.run_args.opts)),
        Some(SubCommand::Run(c)) => (false, c.build_args, Some(c.opts)),
        Some(SubCommand::Test(c)) => (true, c.build_args, Some(c.opts)),
        Some(SubCommand::Build(b)) => (false, b, None),
        Some(SubCommand::Init) => {
            Command::new("git")
                .arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--recursive")
                .spawn()
                .expect("failed to spawn git")
                .wait()
                .expect("failed to wait for get");
            std::env::set_current_dir(ROOT_REPO_PATH.join("common")).expect("failed to set cwd");
            // TODO: re write in rust? move to ci?
            Command::new("./install-toolchain.sh")
                .spawn()
                .expect("failed to spawn install-toolchain.sh")
                .wait()
                .expect("failed to wait for install-toolchain.sh");
            std::process::exit(0);
        }
    };
    println!(
        "Please run `{program} init` (or `cargo run -- init`) the first time you clone the repo and every SafaOS's libstd target update (installs the SafaOS's toolchain)"
    );

    let build_opts = BuildOpts::from_args(tests, &build_args);
    cli::build(build_opts);
    if let Some(opts) = run_opts {
        cli::run(opts, build_opts.output, tests);
    }
}
