use core::ffi::{c_char, CStr};

use alloc::slice;
use bitflags::bitflags;
use macros::display_consts;

use crate::{
    cross_println, kernel,
    memory::paging::{EntryFlags, Page, PageTable},
    VirtAddr,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ElfType(u16);
#[display_consts]
impl ElfType {
    pub const RELOC: ElfType = Self(1);
    pub const EXE: ElfType = Self(2);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ElfInstrSet(u16);

#[display_consts]
impl ElfInstrSet {
    pub const AMD64: Self = Self(0x3E);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ElfIEndianness(u8);

#[display_consts]
impl ElfIEndianness {
    pub const LITTLE: Self = Self(1);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ElfClass(u8);

#[display_consts]
impl ElfClass {
    pub const ELF32: Self = Self(1);
    pub const ELF64: Self = Self(2);
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ElfHeader {
    pub magic: [u8; 4],

    pub class: ElfClass,
    pub endianness: ElfIEndianness,
    pub version: u8,

    pub _osabi: u8,
    pub _abiver: u8,

    pub _padding: [u8; 7],

    pub kind: ElfType,
    //  TODO: this>>
    pub insturction_set: ElfInstrSet,
    pub version_2: u32,

    pub entry_point: VirtAddr,
    pub program_headers_table_offset: usize,
    pub section_header_table_offset: usize,

    pub flags: u32,

    pub size: u16,
    pub program_headers_table_entry_size: u16,
    pub program_headers_table_entries_number: u16,
    pub section_table_entry_size: u16,
    pub section_table_entries: u16,

    pub sections_names_section_offset: u16,
}

#[derive(Debug, Clone, Copy)]
pub enum ElfError {
    UnsupportedClass,
    UnsupportedEndianness,
    UnsupportedKind,
    UnsupportedInsturctionSet,
    NotAnElf,
    NotAnExecutable,
    MapToError,
}

impl ElfHeader {
    #[inline]
    pub fn verify(&self) -> bool {
        self.magic[0] == 0x7F
            && self.magic[1..] == *b"ELF"
            && self.size as usize == size_of::<Self>()
    }

    #[inline]
    pub fn supported(&self) -> Result<(), ElfError> {
        if self.class != ElfClass::ELF64 {
            Err(ElfError::UnsupportedClass)
        } else if self.endianness != ElfIEndianness::LITTLE {
            Err(ElfError::UnsupportedEndianness)
        } else if ![ElfType::EXE, ElfType::RELOC].contains(&self.kind) {
            Err(ElfError::UnsupportedKind)
        } else if self.insturction_set != ElfInstrSet::AMD64 {
            Err(ElfError::UnsupportedInsturctionSet)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Sym {
    pub name_index: u32,
    pub value: VirtAddr,
    pub size: u32,

    pub info: u8,
    pub other: u8,

    pub section_index: u16,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SectionHeader {
    pub name_index: u32,
    pub section_type: u32,
    pub flags: usize,

    pub addr: VirtAddr,
    pub offset: usize,
    pub size: usize,

    pub link: u32,
    pub info: u32,

    pub alignment: usize,
    pub entry_size: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProgramType(u32);
#[display_consts]
impl ProgramType {
    pub const NULL: Self = Self(0);
    pub const LOAD: Self = Self(1);
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct ProgramFlags: u32 {
        const EXEC = 1;
        const WRITE = 2;
        const READ = 4;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProgramHeader {
    pub ptype: ProgramType,
    pub flags: ProgramFlags,
    pub offset: usize,
    pub vaddr: usize,
    pub paddr: usize,
    pub filez: usize,
    pub memz: usize,
    pub align: usize,
}

#[derive(Debug)]
pub struct Elf<'a> {
    pub header: &'a ElfHeader,
    pub sections: &'a [SectionHeader],
    pub program_headers: &'a [ProgramHeader],
}
impl<'a> Elf<'a> {
    #[inline]
    pub fn section_names_table(&self) -> &SectionHeader {
        &self.sections[self.header.sections_names_section_offset as usize]
    }

    pub fn section_names_table_index(&self, name_index: u32) -> &str {
        if name_index == 0 {
            return "";
        }

        let name_table = self.section_names_table();
        let name_ptr = unsafe {
            (self.header as *const ElfHeader as *const u8)
                .add(name_table.offset)
                .add(name_index as usize) as *const c_char
        };

        let str = unsafe { CStr::from_ptr(name_ptr) };
        str.to_str().unwrap()
    }

    #[inline]
    pub fn string_table(&self) -> Option<&SectionHeader> {
        for section in self.sections {
            if self.section_names_table_index(section.name_index) == ".strtab" {
                return Some(section);
            }
        }
        None
    }

    pub fn string_table_index(&self, name_index: u32) -> &str {
        if name_index == 0 {
            return "";
        }

        let str_table = self.string_table().unwrap();
        let str_ptr = unsafe {
            (self.header as *const ElfHeader as *const u8)
                .add(str_table.offset)
                .add(name_index as usize) as *const c_char
        };

        let str = unsafe { CStr::from_ptr(str_ptr) };
        str.to_str().unwrap()
    }

    #[inline]
    pub fn symtable(&self) -> Option<&[Sym]> {
        for section in self.sections {
            if section.section_type == 2 {
                let sym_ptr = unsafe {
                    (self.header as *const ElfHeader as *const u8).add(section.offset) as *const Sym
                };

                let sym_len = section.size / section.entry_size;

                let sym_table = unsafe { slice::from_raw_parts(sym_ptr, sym_len) };
                return Some(sym_table);
            }
        }

        return None;
    }

    pub fn sym_from_value_range(&self, value: VirtAddr) -> Option<Sym> {
        for sym in self.symtable()? {
            if sym.value <= value && (sym.value + sym.size as usize) >= value {
                return Some(*sym);
            }
        }

        return None;
    }

    /// creates an elf from a u8 ptr that lives as long as `bytes`
    pub fn new(bytes: &u8) -> Result<Self, ElfError> {
        let bytes = bytes as *const u8;
        let header_ptr = bytes as *const ElfHeader;

        let header = unsafe {
            if (*header_ptr).verify() {
                &*header_ptr
            } else {
                return Err(ElfError::NotAnElf);
            }
        };

        header.supported()?;

        assert_eq!(
            size_of::<SectionHeader>(),
            header.section_table_entry_size as usize
        );

        assert_eq!(
            size_of::<ProgramHeader>(),
            header.program_headers_table_entry_size as usize
        );

        let section_header_table_ptr =
            unsafe { bytes.add(header.section_header_table_offset) } as *const SectionHeader;

        // TODO: instead make an nth_section function and a section_len function or whateve
        // because section_header_ptr may be unaligned same for programe headers
        assert!(section_header_table_ptr.is_aligned());

        let section_header_table = unsafe {
            slice::from_raw_parts(
                section_header_table_ptr,
                header.section_table_entries as usize,
            )
        };

        let program_headers_table = if header.program_headers_table_offset != 0 {
            let program_headers_table_ptr =
                unsafe { bytes.add(header.program_headers_table_offset) } as *const ProgramHeader;
            assert!(program_headers_table_ptr.is_aligned());
            unsafe {
                slice::from_raw_parts(
                    program_headers_table_ptr,
                    header.program_headers_table_entries_number as usize,
                )
            }
        } else {
            &[]
        };

        Ok(Self {
            header,
            sections: section_header_table,
            program_headers: program_headers_table,
        })
    }

    /// loads an executable elf, map, and copies it to `page_table`
    pub fn load_exec(&self, page_table: &mut PageTable) -> Result<(), ElfError> {
        if self.header.kind != ElfType::EXE {
            return Err(ElfError::NotAnExecutable);
        }

        for header in self.program_headers {
            if header.ptype != ProgramType::LOAD {
                continue;
            }

            let mut entry_flags = EntryFlags::empty();

            if header.flags.contains(ProgramFlags::READ) {
                entry_flags |= EntryFlags::empty();
            }

            if header.flags.contains(ProgramFlags::WRITE) {
                entry_flags |= EntryFlags::WRITABLE;
            }

            if header.flags.contains(ProgramFlags::EXEC) {
                entry_flags |= EntryFlags::empty();
            }

            // assumes ring3
            entry_flags |= EntryFlags::PRESENT | EntryFlags::USER_ACCESSIBLE;

            let page = Page::containing_address(header.vaddr);
            let frame = kernel()
                .frame_allocator()
                .allocate_frame()
                .ok_or(ElfError::MapToError)?;

            page_table
                .map_to(page, frame, entry_flags)
                .ok()
                .ok_or(ElfError::MapToError)?;

            unsafe {
                let mem_start = (frame.start_address | kernel().phy_offset) as *mut u8;

                let file_start = (self.header as *const ElfHeader as *const u8).add(header.offset);

                let mem = slice::from_raw_parts_mut(mem_start, header.memz);
                mem.fill(0);

                let file = slice::from_raw_parts(file_start, header.filez);
                mem[0..file.len()].copy_from_slice(file);
            }
        }
        Ok(())
    }

    pub fn debug(&self) {
        cross_println!("{:#?}", self);
        cross_println!("section names section {:#?}", self.section_names_table());

        for sym in self.symtable().unwrap() {
            cross_println!(
                "sym {}: `{}`",
                sym.name_index,
                self.string_table_index(sym.name_index)
            );
        }

        for section in self.sections {
            cross_println!(
                "section {}: '{}'",
                section.name_index,
                self.section_names_table_index(section.name_index)
            );
        }
    }
}
