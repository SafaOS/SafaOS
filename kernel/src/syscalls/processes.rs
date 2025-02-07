use crate::{
    threading::{self, expose::SpawnFlags},
    utils::errors::ErrorStatus,
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
    path: &str,
    argv: &[&str],
    flags: SpawnFlags,
    dest_pid: Option<&mut usize>,
) -> Result<(), ErrorStatus> {
    let name = name.or(argv.first().map(|v| &**v)).unwrap_or(path);

    let results = threading::expose::pspawn(name, path, argv, flags)?;
    if let Some(dest_pid) = dest_pid {
        *dest_pid = results;
    }
    Ok(())
}
