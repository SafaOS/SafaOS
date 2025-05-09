use clap::Parser;
use cli::{BuildOpts, RunOpts};

#[path = "cli.rs"]
mod cli;

fn main() {
    let args = cli::RunArgs::parse();
    let run_opts = RunOpts::from_args(args.clone(), true);
    let build_opts = BuildOpts::from_args(true, &args.build_args);
    cli::build(build_opts);
    cli::run(run_opts, build_opts.output);
}
