use std::process::Command;

use clap::Parser;
use cli::{BuildOpts, Cli, RunOpts, SubCommand};

mod cli;

fn main() {
    let program = std::env::args().next().unwrap();
    let args = Cli::parse();

    // todo: this is unefficent
    let (tests, build_args, run_opts) = match args.command {
        None => (
            false,
            args.run_args.build_args.clone(),
            Some(RunOpts::from_args(&args.run_args, false)),
        ),
        Some(SubCommand::Run(ref c)) => (
            false,
            c.build_args.clone(),
            Some(RunOpts::from_args(&c, false)),
        ),
        Some(SubCommand::Test(ref c)) => (
            true,
            c.build_args.clone(),
            Some(RunOpts::from_args(&c, true)),
        ),
        Some(SubCommand::Build(b)) => (false, b, None),
        Some(SubCommand::Init { arch }) => {
            Command::new("git")
                .arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--recursive")
                .arg("--depth")
                .arg("1")
                .spawn()
                .expect("failed to spawn git")
                .wait()
                .expect("failed to wait for get");
            safa_builder::rustc::install_safaos_toolchain(arch)
                .expect("failed to install the SafaOS toolchain");
            safa_builder::rustc::reset_userspace();
            safa_builder::initrd::get_fonts().expect("Failed to init fonts");
            std::process::exit(0);
        }
        Some(SubCommand::Reset) => {
            safa_builder::rustc::reset_userspace();
            std::process::exit(0);
        }
        Some(SubCommand::GetFonts) => {
            safa_builder::initrd::get_fonts().expect("Failed to init fonts");
            std::process::exit(0);
        }
        Some(SubCommand::InitFonts) => {
            safa_builder::initrd::init_fonts().expect("Failed to init fonts");
            std::process::exit(0);
        }
    };
    println!(
        "Please run `{program} init` (or `cargo run -- init`) the first time you clone the repo and every SafaOS's libstd target update (installs the SafaOS's toolchain)"
    );

    let build_opts = BuildOpts::from_args(tests, &build_args);
    let iso_path = cli::build(build_opts);
    if let Some(opts) = run_opts {
        cli::run(opts, &iso_path);
    }
}
