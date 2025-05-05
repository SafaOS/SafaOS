use clap::Parser;
use cli::BuildOpts;

#[path = "cli.rs"]
mod cli;

fn main() {
    let args = cli::RunArgs::parse();
    let build_opts = BuildOpts::from_args(true, &args.build_args);
    cli::build(build_opts);
    cli::run(args.opts, build_opts.output, true);
}
