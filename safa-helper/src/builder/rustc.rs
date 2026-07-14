use std::{
    fs::{File, OpenOptions},
    io::{self, BufReader, Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::LazyLock,
};

use flate2::bufread::GzDecoder;
use ring::digest::{Context, SHA256, SHA384, SHA512};

use crate::{
    ROOT_REPO_PATH,
    cargo::cargo_raw,
    log, path_crates, userspace_crates_path,
    utils::{self, ArchTarget},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxABI {
    Musl,
    Gnu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOS {
    Linux(LinuxABI),
    MacOS,
}

impl HostOS {
    pub const fn get_current() -> Option<Self> {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "linux")]  {
                cfg_if::cfg_if! {
                    if #[cfg(target_env = "gnu")] {
                        Some(Self::Linux(LinuxABI::Gnu))
                    } else {
                        Some(Self::Linux(LinuxABI::Musl))
                    }
                }
            }  else if #[cfg(target_os = "macos")] {
                Some(Self::MacOS)
            }
            else {
                None
            }
        }
    }
}

pub const fn get_current_host() -> (ArchTarget, HostOS) {
    (
        ArchTarget::get_host()
            .expect("Unsupported host Arch, Please open an issue with your full target triplet, So I can provide a toolchain for your host, \nor compile it yourself, at https://github.com/SafaOS/safa-rust if have infinite time and is confident you can"),
        HostOS::get_current().expect("Unsupported host OS, Please open an issue with your full target triplet, So I can provide a toolchain for your host, \nor compile it yourself, at https://github.com/SafaOS/safa-rust if have infinite time and is confident you can"),
    )
}
pub fn extract_host(id: &str) -> Option<(ArchTarget, HostOS)> {
    let mut parts = id.split("-");

    let part0 = parts.next();
    let part1 = parts.next();
    let part2 = parts.next();

    let (arch, os) = match (part0, part1, part2) {
        (Some(arch), Some(os), Some(abi)) => {
            let arch = ArchTarget::from_str(arch)?;
            let os = match (os, abi) {
                ("linux", "musl") => HostOS::Linux(LinuxABI::Musl),
                ("linux", "gnu") => HostOS::Linux(LinuxABI::Gnu),
                ("apple", "darwin") => HostOS::MacOS,
                _ => return None,
            };

            (arch, os)
        }
        (Some(arch), Some(os), None) => {
            let arch = ArchTarget::from_str(arch)?;

            let os = match os {
                "linux" => HostOS::Linux(LinuxABI::Gnu),
                "apple" | "macos" => HostOS::MacOS,
                _ => return None,
            };
            (arch, os)
        }
        (Some(os), None, None) => match os {
            "macos" => (ArchTarget::Arm64, HostOS::MacOS),
            _ => return None,
        },
        (None, Some(_), _) | (None, None, Some(_)) => unreachable!(),
        _ => return None,
    };

    Some((arch, os))
}

pub fn toolchain_is_expected(for_arch: ArchTarget, name: &str) -> bool {
    let name = name.trim_end_matches(".tar.gz");
    log!(
        "Checking if {name} is valid for host {:?}, targeting: {:?}",
        get_current_host(),
        for_arch
    );
    let Some((_ver, rest)) = name.split_once("-") else {
        return false;
    };

    let Some((arch_name, rest)) = rest.split_once("-") else {
        return false;
    };

    if ArchTarget::from_str(arch_name) != Some(for_arch) {
        return false;
    }

    let host_id = rest.trim_start_matches("unknown-safaos-");
    extract_host(host_id).is_some_and(|d| d == get_current_host())
}

static COMMON_DIR: LazyLock<PathBuf> = LazyLock::new(|| ROOT_REPO_PATH.join("common"));
static TOOLCHAIN_DIR: LazyLock<PathBuf> = LazyLock::new(|| COMMON_DIR.join("Toolchains"));

/// The latest stable release of the SafaOS target according to common/.latest_stable_release.lock
static LATEST_STABLE_RELEASE: LazyLock<String> = LazyLock::new(|| {
    let path = COMMON_DIR.join(".latest_stable_release.lock");
    std::fs::read_to_string(path)
        .expect("failed to read the latest stable rust version of the SafaOS target")
        .trim()
        .to_string()
});

const TOOLCHAIN_RELEASES_URL: &str = "https://api.github.com/repos/SafaOS/safa-rust/releases";

pub fn safaos_rustc_specifier(arch: ArchTarget) -> String {
    format!("+{}-unknown-safaos", arch.as_str())
}

pub fn rustup_set_toolchain(arch: ArchTarget, path: impl AsRef<Path>) {
    let path = path.as_ref();

    log!(
        "Linking toolchain at: {} as arch: {}",
        path.display(),
        arch.as_str()
    );
    Command::new("rustup")
        .arg("toolchain")
        .arg("link")
        .arg(format!("{}-unknown-safaos", arch.as_str()))
        .arg(path)
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .spawn()
        .expect("failed to spawn rustup")
        .wait()
        .expect("failed to wait for rustup");
}

/// `cargo clean`s all userspace crates.
pub fn reset_userspace() {
    let path = userspace_crates_path(&*ROOT_REPO_PATH);

    log!("Cleaning crates at: {}", path.display());
    let crates = path_crates(&path);
    let mut count = 0;
    for manifest_path in crates.map(|p| p.join("Cargo.toml")).filter(|p| p.exists()) {
        // Reset with and without target_dir set.
        unsafe { std::env::set_var("CARGO_TARGET_DIR", path.join("target")) };
        _ = cargo_raw(
            None,
            ["clean"].into_iter(),
            manifest_path.parent().unwrap(),
            false,
        );

        unsafe { std::env::remove_var("CARGO_TARGET_DIR") };

        _ = cargo_raw(
            None,
            ["clean"].into_iter(),
            manifest_path.parent().unwrap(),
            false,
        );
        count += 1;
    }
    log!("Cleaned {} crates", count);
}

fn algo_for(name: &str) -> Option<&'static ring::digest::Algorithm> {
    match name {
        "sha256" => Some(&SHA256),
        "sha384" => Some(&SHA384),
        "sha512" => Some(&SHA512),
        _ => None,
    }
}

pub fn verify_digest(mut file: &File, digest: &str) -> io::Result<bool> {
    let (algo_name, expected_hex) = digest.split_once(':').expect("Invalid github digest");
    let Some(algo) = algo_for(algo_name) else {
        panic!("unsupported algorithm, digest: {digest}")
    };

    let mut ctx = Context::new(algo);
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    let actual_hex = ctx
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    Ok(actual_hex.eq_ignore_ascii_case(expected_hex))
}

pub fn download_or_cache(
    cache_dir: &Path,
    name: &str,
    url: &str,
    digest: &str,
) -> io::Result<File> {
    _ = std::fs::create_dir_all(cache_dir);
    let cache_path = cache_dir.join(name);

    let open_attempt = OpenOptions::new()
        .create(false)
        .read(true)
        .write(true)
        .open(&cache_path);
    let re_download = move |e: io::Error| {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .truncate(true)
            .open(cache_path)?;

        log!(
            "cache file open failed: {e:?} (expected in case of the first attempt), downloading and caching {}",
            url
        );

        utils::https_get_write(
            url,
            &["Accept: application/octet-stream", "Accept-Encoding: gzip"],
            |data| {
                file.write_all(data).unwrap();
                Ok(data.len())
            },
        )?;

        file.flush()?;
        file.seek(io::SeekFrom::Start(0))?;
        Ok(file)
    };

    match open_attempt {
        Ok(mut f) => {
            if !verify_digest(&f, digest)? {
                return re_download(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "File was found cached but hash verification failed.",
                ));
            } else {
                f.seek(io::SeekFrom::Start(0))?;
                return Ok(f);
            }
        }
        Err(e) => re_download(e),
    }
}

pub fn install_safaos_toolchain(arch: ArchTarget) -> io::Result<()> {
    let api_url = TOOLCHAIN_RELEASES_URL;
    log!("installing the SafaOS toolchain from: {api_url}");

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
    let (download_url, digest, name) = results
        .find(|x| {
            x.get("tag_name").is_some_and(|tag_name| {
                tag_name
                    .as_str()
                    .unwrap()
                    .starts_with(&*LATEST_STABLE_RELEASE)
            })
        })
        .and_then(|x| x.get("assets"))
        .and_then(|assets| assets.as_array())
        .and_then(|assets| {
            assets.iter().find(|x| {
                x.get("name").is_some_and(|name| {
                    toolchain_is_expected(arch, name.as_str().expect("A string for name"))
                })
            })
        })
        .and_then(|x| {
            Some((
                x.get("browser_download_url")?,
                x.get("digest")?,
                x.get("name")?,
            ))
        })
        .and_then(|(x, y, z)| Some((x.as_str()?, y.as_str()?, z.as_str()?)))
        .unwrap_or_else(|| {
            panic!(
                "install_safaos_toolchain: failed to get download URL for version {}",
                &*LATEST_STABLE_RELEASE
            )
        });

    let file;

    let download_dir = TOOLCHAIN_DIR.join("downloads");
    match download_or_cache(&download_dir, name, download_url, digest) {
        Ok(f) => file = f,
        Err(e) => {
            log!(
                "Failed to download toolchain to: {}/{name}, url: {download_url}, digest: {digest}",
                download_dir.display()
            );
            return Err(e);
        }
    }

    let toolchain_root = TOOLCHAIN_DIR.join(name.trim_end_matches(".tar.gz"));
    log!("Deleting and reconstructing: {}", toolchain_root.display());
    _ = std::fs::remove_dir_all(&toolchain_root);
    std::fs::create_dir_all(&toolchain_root).expect("Failed to create toolchain root");

    log!(
        "extracting downloaded file from {}/{name} to {}",
        download_dir.display(),
        toolchain_root.display(),
    );

    let decompressor = GzDecoder::new(BufReader::new(file));

    let mut archive = tar::Archive::new(decompressor);
    archive.set_overwrite(true);
    archive.unpack(&toolchain_root)?;

    rustup_set_toolchain(arch, toolchain_root.join("usr/local"));
    Ok(())
}
