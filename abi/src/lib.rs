#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(any(target_os = "safaos", target_os = "none")))]
compile_error!("abi should only be used for SafaOS or freestanding targets");

pub mod errors;
pub mod io;
pub mod syscalls;
