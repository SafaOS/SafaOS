#![no_std]
pub mod ansi;
pub mod bstr;
pub mod display;
pub mod either;
pub use safa_abi as abi;
pub use safa_abi::consts;
pub use safa_abi::errors;
pub mod io;
pub mod path;
pub mod syscalls;

pub type Name = heapless::String<{ consts::MAX_NAME_LENGTH }>;
