use std::{io, path::Path};

use curl::easy::{List, WriteError};
/// Defines a target architecture to compile SafaOS to
#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum ArchTarget {
    #[value(name = "aarch64")]
    Arm64,
    #[value(name = "x86_64")]
    X86_64,
}

impl Default for ArchTarget {
    fn default() -> Self {
        Self::X86_64
    }
}

impl ArchTarget {
    /// Whether or not the arch has a libstd port
    /// used for architectures currently in development:w
    pub const fn has_rustc_target(&self) -> bool {
        match self {
            Self::X86_64 => true,
            Self::Arm64 => false,
        }
    }
    /// Gets the host architecture
    /// returns None if unsupported
    pub const fn get_host() -> Option<Self> {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")]  {
                Some(Self::X86_64)
            }  else if #[cfg(target_arch = "aarch64")] {
                Some(Self::Arm64)
            }
            else {
                None
            }
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Arm64 => "aarch64",
        }
    }
}

impl ToString for ArchTarget {
    fn to_string(&self) -> String {
        String::from(self.as_str())
    }
}

/// The default architecture for build SafaOS
pub const DEFAULT_ARCH: ArchTarget = if let Some(s) = ArchTarget::get_host() {
    s
} else {
    ArchTarget::X86_64
};
/// make a Get request to a URL and returns the results as a String
pub fn https_get(url: &str, headers: &[&str]) -> std::io::Result<String> {
    let mut response = Vec::new();
    https_get_write(url, headers, |data| {
        response.extend_from_slice(data);
        Ok(data.len())
    })?;

    Ok(String::from_utf8(response).unwrap())
}

/// make a Get request to a URL, writes the result chunk by chunk by calling the write_fn
pub fn https_get_write(
    url: &str,
    headers: &[&str],
    write_fn: impl FnMut(&[u8]) -> Result<usize, WriteError>,
) -> io::Result<()> {
    let mut headers_list = List::new();
    for header in headers {
        headers_list.append(header).unwrap();
    }

    let mut handle = curl::easy::Easy::new();

    handle.useragent(&format!("curl/{}", curl::Version::get().version()))?;
    handle.get(true)?;
    handle.follow_location(true)?;
    handle.url(url)?;
    handle.http_headers(headers_list)?;
    // sub-scope because handle has to be dropped before the response is used
    {
        let mut handle = handle.transfer();
        handle.write_function(write_fn)?;

        handle.perform()?;
    }

    Ok(())
}

pub fn recursive_copy(path: &Path, to_path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::create_dir_all(&to_path)?;
        for entry in path.read_dir()? {
            let entry = entry?;
            let src = path.join(entry.file_name());
            let dest = to_path.join(entry.file_name());
            recursive_copy(&src, &dest)?;
        }
    } else {
        std::fs::copy(&path, &to_path)?;
    }
    Ok(())
}
