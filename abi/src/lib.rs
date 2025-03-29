#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(any(target_os = "safaos", target_os = "none")))]
compile_error!("abi should only be used for SafaOS or freestanding targets");

pub mod errors;
pub mod io;
pub mod syscalls;

pub mod consts {
    // defines the max length for file names and task names
    pub const MAX_NAME_LENGTH: usize = 128;
    // defines the max length for paths
    pub const MAX_PATH_LENGTH: usize = 4096;
}
