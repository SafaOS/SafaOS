#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(any(target_os = "safaos", target_os = "none")))]
compile_error!("abi should only be used for SafaOS or freestanding targets");

pub mod errors;
pub mod ffi;
pub mod fs;
pub mod mem;
pub mod process;
pub mod sockets;
pub mod syscalls;

pub mod consts {
    /// defines the max byte length for file names and process names
    pub const MAX_NAME_LENGTH: usize = 128;
    /// defines the max byte length for paths
    pub const MAX_PATH_LENGTH: usize = 1024;
    /// defines the max byte length for drive names (not counted in [`MAX_PATH_LENGTH`])
    pub const MAX_DRIVE_NAME_LENGTH: usize = 8;
}
