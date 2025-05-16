use std::time::Instant;

use cli::{BuildArgs, BuildOpts, RunArgs, RunOpts};
use safa_builder::utils::ArchTarget;

#[path = "cli.rs"]
mod cli;

const TEST_ARCHS: &[ArchTarget] = &[ArchTarget::Arm64, ArchTarget::X86_64];

fn do_test(args: RunArgs) {
    let run_opts = RunOpts::from_args(&args, true);
    let build_opts = BuildOpts::from_args(true, &args.build_args);
    cli::build(build_opts);
    cli::run(run_opts, build_opts.output);
}

fn arch_test_args(arch: ArchTarget) -> RunArgs {
    RunArgs {
        no_kvm: true,
        no_gui: true,
        debugger: false,
        qemu_args: String::new(),
        build_args: BuildArgs {
            output: Some(format!("out/safa-tests-{}.iso", arch.as_str())),
            verbose: true,
            arch,
        },
    }
}

fn main() {
    for arch in TEST_ARCHS {
        let time = Instant::now();
        safa_builder::log!("Testing arch: {}", arch.as_str());
        let args = arch_test_args(*arch);
        do_test(args);
        safa_builder::log!(
            "Testing arch '{}' done, elapsed: {}ms",
            arch.as_str(),
            time.elapsed().as_millis()
        );
    }
}
