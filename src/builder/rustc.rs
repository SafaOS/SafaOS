use std::{
    ffi::OsString,
    io::{self, BufReader, Seek, Write},
    os::unix::ffi::OsStringExt,
    path::PathBuf,
    process::Command,
    sync::LazyLock,
};

use flate2::bufread::GzDecoder;
use tempfile::NamedTempFile;

use crate::{
    ROOT_REPO_PATH, log,
    utils::{self, ArchTarget},
};

/// The latest supported stable Rust version of the SafaOS target according to common/.latest_stable_rustc_version.lock
static LATEST_SUPPORTED_STABLE_RUSTC_VERSION: LazyLock<String> = LazyLock::new(|| {
    let path = ROOT_REPO_PATH.join("common/.latest_stable_rustc_version.lock");
    let string = std::fs::read_to_string(path)
        .expect("failed to read the latest stable rust version of the SafaOS target");
    string.trim().to_string()
});

/// The latest stable release of the SafaOS target according to common/.latest_stable_release.lock
static LATEST_STABLE_RELEASE: LazyLock<String> = LazyLock::new(|| {
    let path = ROOT_REPO_PATH.join("common/.latest_stable_release.lock");
    std::fs::read_to_string(path)
        .expect("failed to read the latest stable rust version of the SafaOS target")
        .trim()
        .to_string()
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

static SYSROOT: LazyLock<PathBuf> = LazyLock::new(|| {
    let specifier = &*SAFAOS_RUSTC_SPECIFIER;

    let mut cmd = Command::new("rustc");
    if !specifier.is_empty() {
        cmd.arg(specifier);
    }

    let mut output = cmd
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("failed to get the sysroot")
        .stdout;
    output.pop_if(|c| *c == b'\n');

    let path = PathBuf::from(OsString::from_vec(output));
    path
});

static TOOLCHAIN_SYSROOT: LazyLock<PathBuf> = LazyLock::new(|| SYSROOT.join("lib/rustlib/"));
fn toolchain_root(arch: ArchTarget) -> PathBuf {
    TOOLCHAIN_SYSROOT.join(format!("{}-unknown-safaos", arch.as_str()))
}

pub fn install_safaos_toolchain(arch: ArchTarget) -> io::Result<()> {
    log!("installing the SafaOS toolchain");
    let api_url = "https://api.github.com/repos/SafaOS/rust/releases";

    let response = utils::https_get(
        api_url,
        &[
            "Accept: application/vnd.github+json",
            "X-GitHub-Api-Version: 2022-11-28",
        ],
    )?;

    let response_json: Vec<serde_json::Value> = serde_json::from_str(&response)?;

    let mut results = response_json.iter();
    // FIXME: might be a little bit ugly
    let download_url = results
        .find(|x| {
            x.get("tag_name").is_some_and(|tag_name| {
                tag_name
                    .as_str()
                    .unwrap()
                    .starts_with(&*LATEST_STABLE_RELEASE)
            })
        })
        .and_then(|x| x.get("assets"))
        .and_then(|x| x.as_array())
        .and_then(|x| {
            x.iter().find(|x| {
                x.get("name")
                    .is_some_and(|name| name.as_str().unwrap().contains(arch.as_str()))
            })
        })
        .and_then(|x| x.get("browser_download_url"))
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| {
            panic!(
                "install_safaos_toolchain: failed to get download URL for version {}",
                &*LATEST_STABLE_RELEASE
            )
        });

    log!("downloading {}", download_url);

    let mut file = NamedTempFile::new()?;
    utils::https_get_write(
        download_url,
        &["Accept: application/octet-stream", "Accept-Encoding: gzip"],
        |data| {
            file.write_all(data).unwrap();
            Ok(data.len())
        },
    )?;

    file.flush()?;
    file.seek(io::SeekFrom::Start(0))?;

    let toolchain_root = toolchain_root(arch);
    log!(
        "extracting downloaded file from {} to {}",
        file.path().display(),
        toolchain_root.display(),
    );

    let decompressor = GzDecoder::new(BufReader::new(file));

    let mut archive = tar::Archive::new(decompressor);
    archive.set_overwrite(true);
    archive.unpack(toolchain_root)?;

    // let extracted_path = toolchain_root;
    // // recursively copy extracted_path to toolchain_root
    // utils::recursive_copy(&extracted_path, toolchain_root)?;
    // std::fs::remove_dir_all(extracted_path)?;
    Ok(())
}
