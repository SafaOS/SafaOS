use core::ffi::CStr;

use crate::{
    bootloader,
    logging::{self, serial::_REGISTER_LOGGER},
    oninit,
};

#[derive(Debug, Clone, Copy, PartialEq)]
/// A .sym file
pub struct SymFile<'a> {
    count: usize,
    addresses: &'a [u64],
    offsets: &'a [u64],
    sym_data: &'a [u8],
}

impl<'a> SymFile<'a> {
    /// Parsers a .sym file.
    ///
    /// Returns None if not enough data in file.
    ///
    /// Safety: The file is not verified enough, call [`Self::verify`] and make sure it is true for operations on it to not be UB.
    #[inline]
    pub unsafe fn parse(data: &'a [u8]) -> Option<Self> {
        let data_ptr = data.as_ptr();
        let count = u64::from_le_bytes(*data.first_chunk::<8>()?) as usize;
        unsafe {
            let addresses_ptr = data_ptr.byte_add(8);
            let offsets_ptr = data_ptr.byte_add(8 + (count * 8));
            let sym_data = data.get((8 + (count * 2 * 8))..)?;

            Some(Self {
                count,
                addresses: core::slice::from_raw_parts(addresses_ptr.cast(), count),
                offsets: core::slice::from_raw_parts(offsets_ptr.cast(), count),
                sym_data,
            })
        }
    }

    /// Verifies that all offsets and strings are correct within a .sym file.
    pub fn verify(&self) -> Result<(), (usize, usize, &'static str)> {
        for (i, off) in self.offsets.iter().enumerate() {
            let off = u64::from_le(*off) as usize;
            let Some(bytes) = self.sym_data.get(off..) else {
                return Err((i, off, "Offset Out Of Range"));
            };

            let Ok(str) = CStr::from_bytes_until_nul(bytes) else {
                return Err((i, off, "No null terminator"));
            };

            if core::str::from_utf8(str.to_bytes()).is_err() {
                return Err((i, off, "Invalid UTF-8"));
            }

            if let Some(prev_i) = i.checked_sub(1) {
                let prev_off = u64::from_le(self.offsets[prev_i]);
                if prev_off as usize > off {
                    return Err((i, off, "Offsets Not Sorted"));
                }
            }
        }

        Ok(())
    }

    /// Gets the name of symbols
    ///
    /// UB / Panick if corrupted, please [`Self::verify`] once before use.
    pub fn get_name_of(&self, addr: u64) -> Option<&'a str> {
        for (i, a) in self.addresses.iter().enumerate() {
            let prev = i.checked_sub(1).map(|prev_i| {
                (
                    u64::from_le(self.offsets[prev_i]),
                    u64::from_le(self.addresses[prev_i]),
                )
            });

            let off = u64::from_le(self.offsets[i]) as usize;
            let curr_addr = u64::from_le(*a);

            if let Some((prev_off, prev_addr)) = prev
                && prev_addr < addr
                && curr_addr > addr
            {
                return Some(
                    core::str::from_utf8(
                        CStr::from_bytes_until_nul(&self.sym_data[prev_off as usize..])
                            .expect("No null terminator")
                            .to_bytes(),
                    )
                    .expect("Invalid utf8"),
                );
            }

            if curr_addr >= addr {
                return Some(
                    core::str::from_utf8(
                        CStr::from_bytes_until_nul(&self.sym_data[off..])
                            .expect("No null terminator")
                            .to_bytes(),
                    )
                    .expect("Invalid utf8"),
                );
            }
        }

        None
    }
}

oninit::define! {
    pub unsafe static KERNEL_SYMS: Option<SymFile<'static>> = with _REGISTER_LOGGER || 'blk: {
        let Some(data) = bootloader::symbols_file() else {
            logging::warn!(SymFile, "No kernel .sym file found");
            break 'blk None;
        };

        if let Some(data) = unsafe { SymFile::parse(data) } {
            if let Err((i, off, e)) = data.verify() {
                logging::warn!(SymFile, "Corrupted kernel .sym file, symbol {i} off={off:#x}: {e}");
            }

            logging::info!(SymFile, "Loaded {} symbols", data.count);
            Some(data)
        } else {
            logging::warn!(SymFile, "Corrupted kernel .sym file");
            None
        }
    };
}
