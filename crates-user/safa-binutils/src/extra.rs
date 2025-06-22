#[path = "extra/json.rs"]
pub mod json;
#[path = "extra/usb_json.rs"]
mod usb_json;

use std::ffi::OsString;

/// Converts an ossting to String
/// Always safe because in SafaOS Strings layout matches the rust Strings layout
#[inline(always)]
pub fn ostring_to_string(os: OsString) -> String {
    let bytes = os.into_encoded_bytes();
    unsafe { String::from_utf8_unchecked(bytes) }
}

#[macro_export]
macro_rules! tri_io {
    ($expr: expr) => {
        $crate::tri!($expr.map_err(|e| safa_api::errors::err_from_io_error_kind(e.kind())))
    };
}

#[macro_export]
macro_rules! tri {
    ($expr: expr) => {
        match $expr {
            Ok(data) => data,
            Err(e) => return safa_api::errors::SysResult::Error(e),
        }
    };
}
