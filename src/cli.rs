use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

use clap::Parser;
use safa_builder::{
    Builder, ROOT_REPO_PATH,
    utils::{self, ArchTarget},
};

const fn get_qemu(arch: ArchTarget) -> &'static str {
    match arch {
        ArchTarget::Arm64 => "qemu-system-aarch64",
        ArchTarget::X86_64 => "qemu-system-x86_64",
    }
}

fn get_ovmf(arch: ArchTarget) -> PathBuf {
    let path = format!("common/ovmf-code-{}.fd", arch.as_str());
    ROOT_REPO_PATH.join(path)
}

#[derive(Clone, Copy, Debug)]
pub struct RunOpts<'a> {
    /// runs with kvm disabled
    no_kvm: bool,
    /// runs with gui disabled
    no_gui: bool,
    /// runs with debugger enabled on port 1234
    debugger: bool,
    tests: bool,
    arch: ArchTarget,
    qemu_args: &'a str,
}

impl<'a> RunOpts<'a> {
    pub fn from_args(args: &'a RunArgs, tests: bool) -> Self {
        Self {
            no_gui: args.no_gui,
            no_kvm: args.no_kvm,
            debugger: args.debugger,
            tests,
            arch: args.build_args.arch,
            qemu_args: &args.qemu_args,
        }
    }
}

#[derive(Parser, Debug, Clone)]
pub struct BuildArgs {
    #[arg(short, long)]
    /// The final output of the built iso the default is out/safaos.iso for normal isos and out/safaos-tests.iso for test isos
    pub output: Option<String>,
    #[arg(short, long, default_value = "false")]
    pub verbose: bool,
    #[arg(short, long, default_value_t = utils::DEFAULT_ARCH)]
    pub arch: ArchTarget,
}

#[derive(Debug, Clone, Copy)]
pub struct BuildOpts<'a> {
    pub output: &'a str,
    pub verbose: bool,
    pub tests: bool,
    pub target_arch: ArchTarget,
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
            target_arch: value.arch,
        }
    }
}

#[derive(Parser, Debug, Clone)]
pub struct RunArgs {
    #[arg(long, default_value = "false")]
    /// runs with kvm disabled
    no_kvm: bool,
    #[arg(long, default_value = "false")]
    /// runs with gui disabled
    no_gui: bool,
    #[arg(long, default_value = "false")]
    /// runs with debugger enabled on port 1234
    debugger: bool,
    #[arg(long, default_value = "")]
    qemu_args: String,
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
    Builder::create(opts.output, opts.target_arch)
        .set_testing(opts.tests)
        .set_verbose(opts.verbose)
        .build()
        .expect("build failed")
}

/// Runs qemu with options `opts` and iso at `path`, if `tests` is true, will scan output for tests failure or success
pub fn run(opts: RunOpts, path: &str) {
    let qemu = get_qemu(opts.arch);
    let path_to_ovmf = get_ovmf(opts.arch);

    let mut cmd = Command::new(qemu);

    cmd.arg("-cdrom")
        .arg(path)
        .arg("-serial")
        .arg("stdio")
        .arg("-m")
        .arg("2G")
        .arg("-drive")
        .arg(format!(
            "if=pflash,unit=0,format=raw,file={},readonly=on",
            path_to_ovmf.display()
        ));

    let arch_args: &[&str] = match opts.arch {
        // FIXME: unefficent and can be written better
        ArchTarget::Arm64 => &[
            "-M",
            "virt",
            "-cpu",
            "cortex-a72",
            "-device",
            "qemu-xhci",
            "-device",
            "usb-kbd",
            "-device",
            "ramfb",
        ],
        ArchTarget::X86_64 => &[],
    };

    cmd.args(arch_args);

    if !opts.no_kvm {
        cmd.arg("-enable-kvm");
    }

    if opts.no_gui {
        cmd.arg("-display").arg("none");
    }

    if opts.debugger {
        cmd.arg("-s").arg("-S");
    }

    if !opts.qemu_args.is_empty() {
        cmd.args(opts.qemu_args.split(|c: char| c.is_whitespace()));
    }
    if opts.tests {
        cmd.stdout(Stdio::piped());
    }
    println!("--------------   QEMU OUTPUT   --------------");
    println!();
    let output = cmd
        .spawn()
        .unwrap_or_else(|_| panic!("{} required to run", qemu))
        .wait_with_output()
        .expect("failed to wait for qemu to exit");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // if tests is on we read stdout for scanning later so it is piped so we have to echo it...
    if opts.tests {
        print!("{}", stdout);
    }
    println!();
    println!("-------------- END QEMU OUTPUT --------------");

    if !output.status.success() {
        eprintln!("qemu exited with {}", output.status);
        std::process::exit(-1);
    }

    if opts.tests {
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
