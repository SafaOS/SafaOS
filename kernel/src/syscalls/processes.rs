use alloc::{format, string::ToString};

use crate::{
    threading::{self, expose::SpawnFlags},
    utils::{errors::ErrorStatus, path::Path},
};

pub fn syswait(pid: usize, dest_code: Option<&mut usize>) -> Result<(), ErrorStatus> {
    let code = threading::expose::wait(pid);
    if let Some(dest_code) = dest_code {
        *dest_code = code;
    }
    Ok(())
}

pub fn syspspawn(
    name: Option<&str>,
    path: Path,
    argv: &[&str],
    flags: SpawnFlags,
    dest_pid: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let name = name.map(|s| s.to_string());
    // we are using _else because it is expensive to allocate all of this
    let name = name
        .or_else(|| argv.first().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{path}"));

    let results = threading::expose::pspawn(name, path, argv, flags)?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}
