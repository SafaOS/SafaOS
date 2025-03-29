use crate::utils::types::Name;
use core::fmt::Write;

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
    let name = match name {
        Some(raw) => Name::try_from(raw).map_err(|()| ErrorStatus::StrTooLong)?,
        None => {
            let mut name = Name::new();
            _ = name.write_fmt(format_args!("{path}"));
            name
        }
    };

    let results = threading::expose::pspawn(name, path, argv, flags)?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}
