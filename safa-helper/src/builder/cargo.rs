//! a helper api over the cargo cli
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{rustc, utils::ArchTarget};

use super::log;
use cargo_metadata::Message;

/// Rustc target
pub enum RustcTarget {
    None,
    SafaOS,
}

impl RustcTarget {
    const fn into_cargo_target(self, arch: ArchTarget) -> &'static str {
        match (self, arch) {
            (Self::SafaOS, ArchTarget::X86_64) => "x86_64-unknown-safaos",
            (Self::SafaOS, ArchTarget::Arm64) => "aarch64-unknown-safaos",
            (Self::None, ArchTarget::X86_64) => "x86_64-unknown-none",
            (Self::None, ArchTarget::Arm64) => "aarch64-unknown-none",
        }
    }
}

pub fn cargo_raw<'a, I>(
    target: Option<(ArchTarget, RustcTarget)>,
    args: I,
    work_dir: impl AsRef<Path>,
    parse_diagnostics: bool,
) -> impl Iterator<Item = Message>
where
    I: Iterator<Item = &'a str>,
{
    let mut command = Command::new("cargo");
    command.args(args);

    if let Some((arch, target)) = target {
        command.arg("--target").arg(target.into_cargo_target(arch));
    }

    if parse_diagnostics {
        command.arg("--message-format=json-render-diagnostics");
    }

    let output = command
        .current_dir(work_dir)
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to execute cargo");

    if !output.status.success() {
        // TODO: add and use errors
        panic!("cargo: exited with status {}", output.status);
    }

    let reader = std::io::Cursor::new(output.stdout);
    let results = Message::parse_stream(reader);
    let results = results.filter_map(|s| s.ok());
    results
}

/// Builds a crate and returns the path to the full path of the executable and the executable name.
///
/// the build subcommand is not supplied you are expected to give it to args
///
/// panicks if the build fails
fn cargo_build_and_get_exe<'a, I>(
    crate_path: &Path,
    arch: ArchTarget,
    target: RustcTarget,
    args: I,
) -> Vec<(PathBuf, String)>
where
    I: Iterator<Item = &'a str>,
{
    assert!(crate_path.is_absolute() && crate_path.exists());
    let manifest_path = crate_path.join("Cargo.toml");
    assert!(manifest_path.exists());

    let cwd = std::env::current_dir().expect("failed to get current directory");
    // because cargo doesn't have a stable -C flag
    std::env::set_current_dir(crate_path).expect("failed to set current directory");

    let results: Vec<Message> = cargo_raw(Some((arch, target)), args, &crate_path, true).collect();
    let results = results.into_iter();

    let results = results.filter(|message| match message {
        Message::CompilerArtifact(_) => true,
        Message::BuildFinished(_) => true,
        _ => false,
    });

    macro_rules! only_if_artifact {
        ($message: expr, $otherwise:expr) => {{
            let Message::CompilerArtifact(r) = $message else {
                return $otherwise;
            };
            r
        }};
        ($message: expr) => {
            only_if_artifact!($message, true)
        };
    }

    let results = results.filter(|r| only_if_artifact!(r).target.is_bin());
    let mut results = results.filter(|r| only_if_artifact!(r).executable.is_some());

    let build_status = results
        .next_back()
        .expect("failed to get the build status from cargo diagnostics");

    let Message::BuildFinished(build_status) = build_status else {
        panic!("Unexpected: multiple executables in cargo diagnostics")
    };
    assert!(build_status.success, "Build Failed");

    let compiler_artifacts = results.filter_map(|r| match r {
        Message::CompilerArtifact(compiler_artifact) => Some((
            compiler_artifact.executable.unwrap(),
            compiler_artifact.target.name,
        )),
        _ => None,
    });

    let compiler_artifacts: Vec<(PathBuf, String)> = compiler_artifacts
        .map(|(path, name)| (path.into(), name))
        .collect();

    // return back to the previous directory
    std::env::set_current_dir(cwd).expect("failed to set current directory");
    compiler_artifacts
}

/// Builds a binary crate for SafaOS
/// panicls if the build fails
/// returns the path to the executable and the name of the binary
pub fn build_safaos(
    crate_path: &Path,
    arch: ArchTarget,
    args: &[&'static str],
) -> Vec<(PathBuf, String)> {
    let specifier = rustc::safaos_rustc_specifier(arch);

    log!(
        "Compiling crate at {} (SafaOS) with rust `{}`",
        crate_path.display(),
        specifier
    );
    let args = args.iter();
    let args = args.map(|s| *s);

    let args = [specifier.as_str(), "build"]
        .into_iter()
        .into_iter()
        .chain(args);
    // if specifier is empty it'd be a problem
    let args = args.filter(|x| !x.is_empty());

    let results = cargo_build_and_get_exe(crate_path, arch, RustcTarget::SafaOS, args);
    log!("Successful, got {} executables", results.len());
    results
}

/// Builds a binary crate for the freestanding target (currently only x86_64-unknown-none)
/// panicks if the build fails
/// returns the path to the executable and the name of the binary
pub fn build_freestanding(
    crate_path: &Path,
    arch: ArchTarget,
    args: &[&'static str],
) -> Vec<(PathBuf, String)> {
    log!(
        "Compiling crate at {} (freestanding)...",
        crate_path.display()
    );
    let args = args.iter();
    let args = args.map(|s| *s);
    let args = ["build"].into_iter().chain(args);
    let results = cargo_build_and_get_exe(crate_path, arch, RustcTarget::None, args);
    log!("Successful, got {} executables", results.len());
    results
}

/// Builds a binary crate with tests enabled for the freestanding target (currently only x86_64-unknown-none)
/// panicks if the build fails
/// returns the path to the executable and the name of the binary
pub fn build_tests_freestanding(
    crate_path: &Path,
    arch: ArchTarget,
    args: &[&'static str],
) -> Vec<(PathBuf, String)> {
    log!(
        "compiling crate at {} (freestanding)...",
        crate_path.display()
    );
    let args = args.iter();
    let args = args.map(|s| *s);
    let args = ["test", "--no-run"].into_iter().chain(args);
    let results = cargo_build_and_get_exe(crate_path, arch, RustcTarget::None, args);
    log!("Successful, got {} executables", results.len());
    results
}
