use crate::{
    threading::{self, expose::SpawnFlags, processes::ProcessInfo},
    utils::{
        errors::ErrorStatus,
        ffi::{Optional, Required, Slice, SliceMut},
    },
};

#[no_mangle]
extern "C" fn syswait(pid: usize) -> usize {
    threading::expose::wait(pid)
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SpawnConfig {
    pub name: Slice<u8>,
    pub argv: SliceMut<Slice<u8>>,
    pub flags: SpawnFlags,
}

impl SpawnConfig {
    pub fn as_rust(&self) -> (&str, &[&str], SpawnFlags) {
        (self.name.into_str(), self.argv.into_str_slice(), self.flags)
    }
}

// if dest_pid is null we will just ignore it
#[no_mangle]
extern "C" fn sysspawn(
    elf_ptr: *const u8,
    elf_len: usize,
    config: Required<SpawnConfig>,
    dest_pid: Optional<usize>,
) -> ErrorStatus {
    let config = config.get()?;
    let (name, argv, flags) = config.as_rust();
    let elf_bytes = Slice::new(elf_ptr, elf_len)?.into_slice();
    match threading::expose::spawn(name, elf_bytes, argv, flags) {
        Err(err) => err.into(),
        Ok(pid) => {
            if let Some(dest_pid) = dest_pid.into_option() {
                *dest_pid = pid
            }
            ErrorStatus::None
        }
    }
}

#[no_mangle]
extern "C" fn syspspawn(
    path_ptr: *const u8,
    path_len: usize,
    config: Required<SpawnConfig>,
    dest_pid: Optional<usize>,
) -> ErrorStatus {
    let config = config.get()?;
    let path = Slice::new(path_ptr, path_len)?.into_str();
    let (name, argv, flags) = config.as_rust();

    match threading::expose::pspawn(name, path, argv, flags) {
        Err(err) => err.into(),
        Ok(pid) => {
            if let Some(dest_pid) = dest_pid.into_option() {
                *dest_pid = pid;
            }
            ErrorStatus::None
        }
    }
}

#[no_mangle]
extern "C" fn syspcollect(ptr: *mut ProcessInfo, len: usize) -> ErrorStatus {
    let slice = SliceMut::new(ptr, len)?.into_slice();

    if let Err(()) = threading::expose::pcollect(slice) {
        ErrorStatus::Generic
    } else {
        ErrorStatus::None
    }
}
