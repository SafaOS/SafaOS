use std::{process::Command, sync::LazyLock};

use crate::ROOT_REPO_PATH;

static LATEST_SUPPORTED_STABLE_RUSTC_VERSION: LazyLock<String> = LazyLock::new(|| {
    let path = ROOT_REPO_PATH.join("common/.latest_stable_rustc_version.lock");
    std::fs::read_to_string(path)
        .expect("failed to read the latest stable rust version of the SafaOS target")
});

static CURRENT_RUSTC_VERSION: LazyLock<String> = LazyLock::new(|| {
    let output = Command::new("rustc")
        .arg("--version")
        .output()
        .expect("failed to get the current rustc version")
        .stdout;

    let version = output.split(|byte| *byte == b' ').nth(1).unwrap();
    String::from_utf8(version.to_vec()).unwrap()
});

static STABLE_RUSTC_VERSION: LazyLock<String> = LazyLock::new(|| {
    let output = Command::new("rustc")
        .arg("+stable")
        .arg("--version")
        .output()
        .expect("failed to get the +stable rustc version")
        .stdout;

    let version = output.split(|byte| *byte == b' ').nth(1).unwrap();
    String::from_utf8(version.to_vec()).unwrap()
});

/// The specifier to be passed to cargo or rustc
/// so that it can be used build stuff that targets SafaOS
/// this specifier specifies the rust version of the SafaOS target
pub static SAFAOS_RUSTC_SPECIFIER: LazyLock<String> = LazyLock::new(|| {
    let latest_supported_stable_rustc_version = LATEST_SUPPORTED_STABLE_RUSTC_VERSION.trim();
    match (
        CURRENT_RUSTC_VERSION.trim() == latest_supported_stable_rustc_version,
        STABLE_RUSTC_VERSION.trim() == latest_supported_stable_rustc_version,
    ) {
        (true, true) | (true, false) => String::new(),
        (false, true) => String::from("+stable"),
        (false, false) => format!("+{}", latest_supported_stable_rustc_version),
    }
});
