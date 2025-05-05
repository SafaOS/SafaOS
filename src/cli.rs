use std::process::{Command, Stdio};

use clap::Parser;
use safa_builder::{Builder, ROOT_REPO_PATH};

const QEMU: &str = "qemu-system-x86_64";
#[derive(Parser, Clone, Copy, Debug)]
pub struct RunOpts {
    #[arg(long, default_value = "false")]
    /// runs with kvm disabled
    pub no_kvm: bool,
    #[arg(long, default_value = "false")]
    /// runs with gui disabled
    pub no_gui: bool,
    #[arg(long, default_value = "false")]
    /// runs with debugger enabled on port 1234
    pub debugger: bool,
}

#[derive(Parser, Debug)]
pub struct BuildArgs {
    #[arg(short, long)]
    /// The final output of the built iso the default is out/safaos.iso for normal isos and out/safaos-tests.iso for test isos
    pub output: Option<String>,
    #[arg(short, long, default_value = "false")]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct BuildOpts<'a> {
    pub output: &'a str,
    pub verbose: bool,
    pub tests: bool,
}

impl<'a> BuildOpts<'a> {
    pub fn from_args(tests: bool, value: &'a BuildArgs) -> Self {
        Self {
            output: value.output.as_ref().map(|s| &**s).unwrap_or(if tests {
                "out/safaos-tests.iso"
            } else {
                "out/safaos.iso"
            }),
            verbose: value.verbose,
            tests,
        }
    }
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    #[command(flatten)]
    pub opts: RunOpts,
    #[command(flatten)]
    pub build_args: BuildArgs,
}
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<SubCommand>,
    #[command(flatten)]
    pub run_args: RunArgs,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub enum SubCommand {
    /// Initializes the submodules and installs the SafaOS's toolchain (rustc target)
    Init,
    /// Builds a SafaOS iso
    Build(BuildArgs),
    /// Builds and Runs a normal SafaOS iso, requires qemu (default)
    Run(RunArgs),
    /// Builds and runs a test SafaOS iso, requires qemu
    Test(RunArgs),
}

pub fn build(opts: BuildOpts) {
    Builder::create(opts.output)
        .set_testing(opts.tests)
        .set_verbose(opts.verbose)
        .build()
}

/// Runs qemu with options `opts` and iso at `path`, if `tests` is true, will scan output for tests failure or success
pub fn run(opts: RunOpts, path: &str, tests: bool) {
    let mut cmd = Command::new(QEMU);
    cmd.arg("-drive")
        .arg(format!("format=raw,file={}", path))
        .arg("-serial")
        .arg("stdio")
        .arg("-m")
        .arg("512M")
        .arg("-bios")
        .arg(ROOT_REPO_PATH.join("common/OVMF-pure-efi.fd"));

    if !opts.no_kvm {
        cmd.arg("-enable-kvm");
    }

    if opts.no_gui {
        cmd.arg("-display").arg("none");
    }

    if opts.debugger {
        cmd.arg("-s").arg("-S");
    }

    if tests {
        cmd.stdout(Stdio::piped());
    }
    println!("--------------   QEMU OUTPUT   --------------");
    println!();
    let output = cmd
        .spawn()
        .unwrap_or_else(|_| panic!("{} required to run", QEMU))
        .wait_with_output()
        .expect("failed to wait for qemu to exit");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // if tests is on we read stdout for scanning later so it is piped so we have to echo it...
    if tests {
        print!("{}", stdout);
    }
    println!();
    println!("-------------- END QEMU OUTPUT --------------");

    if !output.status.success() {
        eprintln!("qemu exited with {}", output.status);
        std::process::exit(-1);
    }

    if tests {
        let failure_message = b"kernel panic";
        // read tests output for failure
        if stdout
            .as_bytes()
            .windows(failure_message.len())
            .any(|x| x == failure_message)
        {
            eprintln!("tests failed!");
            std::process::exit(-1);
        } else {
            eprintln!("tests successful!");
        }
    }
}
