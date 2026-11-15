use std::collections::HashMap;
use std::fmt::Display;
use std::fs::File;
use std::path::Path;

use elf::abi;
use object::build::elf::Builder;

use crate::log;

use super::Result;

#[derive(Debug)]
pub struct Symbol<'a> {
    addr: u64,
    name: rustc_demangle::Demangle<'a>,
}

impl Symbol<'_> {
    #[inline(always)]
    /// Returns the name of this symbol
    pub fn name<'s>(&'s self) -> impl Display + 's {
        &self.name
    }

    #[inline(always)]
    /// Address of the symbol in the binary
    pub fn address(&self) -> u64 {
        self.addr
    }
}

pub fn parse_symbols(
    binary_path: impl AsRef<Path>,
    mut with_sym: impl FnMut(Symbol),
) -> Result<()> {
    let binary_file = std::fs::File::open(binary_path)?;

    let mut binary = elf::ElfStream::<elf::endian::AnyEndian, _>::open_stream(binary_file)?;
    let Some((symbol_table, str_table)) = binary.symbol_table()? else {
        return Ok(());
    };

    for sym in symbol_table.iter() {
        let name = str_table.get(sym.st_name as usize)?;
        with_sym(Symbol {
            addr: sym.st_value,
            name: rustc_demangle::demangle(name),
        })
    }
    Ok(())
}

/// Creates a .sym file for all the symbols
///
/// u64 symbols count = n
/// The format is the following:
/// [for n] u64 symbol address
/// [for n] u64 symbol names offset relative the the below raw symbols table
///
/// ...null terminated UTF8 symbol names
pub fn create_symbol_file(
    binary_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<()> {
    use std::io::Write;

    let mut addresses = HashMap::new();
    let mut output_buffer = Vec::with_capacity(1024 * 64);
    let mut output_file = File::create(output_path)?;

    let mut count = 0u64;
    parse_symbols(binary_path, |sym| {
        if addresses.contains_key(&sym.address()) {
            return;
        }

        addresses.insert(sym.address(), output_buffer.len());
        write!(output_buffer, "{:#}\0", sym.name()).expect("Failed to accumulate a symbol");
        count += 1;
    })?;

    let mut nums = addresses.into_iter().collect::<Vec<_>>();
    nums.sort_by_key(|(a, _)| *a);

    output_file.write_all(&count.to_le_bytes())?;

    for (k, _) in nums.iter() {
        output_file.write_all(&k.to_le_bytes())?;
    }

    for (_, v) in nums.iter() {
        output_file.write_all(&(*v as u64).to_le_bytes())?;
    }

    output_file.write_all(&output_buffer)?;
    log!("Encoded: {} symbols", count);
    Ok(())
}

/// Strips all the moved symbol information in an elf binary creating a new one
pub fn strip_symbols(input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<()> {
    let data = std::fs::read(input.as_ref())?;
    let mut builder = Builder::read(&*data)?;

    let mut strtabs = Vec::with_capacity(1);
    for section in builder.sections.iter_mut() {
        if section.sh_info == abi::SHT_SYMTAB {
            if let Some(link_section) = section.sh_link_section {
                strtabs.push(link_section);
            }
            section.delete = true;
        }
    }

    for section_id in strtabs {
        let strtab = builder.sections.get_mut(section_id);
        strtab.delete = true;
    }

    builder.delete_orphans();

    let mut out = Vec::new();
    builder.write(&mut out)?;
    std::fs::write(output.as_ref(), out)?;
    Ok(())
}
