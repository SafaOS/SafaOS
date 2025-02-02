use alloc::vec;
use alloc::{slice, string::String, vec::Vec};
use bitflags::bitflags;
use macros::display_consts;
use spin::once::Once;

use crate::{
    memory::{
        copy_to_userspace, frame_allocator,
        paging::{EntryFlags, MapToError, Page, PageTable, PAGE_SIZE},
    },
    utils::errors::{ErrorStatus, IntoErr},
    VirtAddr,
};

use super::io::Readable;

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

impl IntoErr for ElfError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::NotAnExecutable | Self::NotAnElf => ErrorStatus::NotExecutable,
            Self::MapToError => ErrorStatus::MMapError,
            Self::UnsupportedKind
            | Self::UnsupportedInsturctionSet
            | Self::UnsupportedClass
            | Self::UnsupportedEndianness => ErrorStatus::NotSupported,
        }
    }
}

impl From<MapToError> for ElfError {
    fn from(_: MapToError) -> Self {
        Self::MapToError
    }
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
    /// offset from the beginning of the file of the section data
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
pub struct Elf<'a, T: Readable> {
    header: ElfHeader,
    names_table: Once<Option<SectionHeader>>,
    strings_table: Once<Option<SectionHeader>>,
    symbols: Once<Option<Vec<Sym>>>,
    reader: &'a T,
}

struct SectionHeaderIter<'a, T: Readable> {
    elf: &'a Elf<'a, T>,
    current: usize,
}

impl<'a, T: Readable> Iterator for SectionHeaderIter<'a, T> {
    type Item = SectionHeader;

    fn next(&mut self) -> Option<Self::Item> {
        let section = self.nth(self.current);
        self.current += 1;
        section
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.elf.get_section(n)
    }
}

struct ProgramHeaderIter<'a, T: Readable> {
    elf: &'a Elf<'a, T>,
    current: usize,
}

impl<'a, T: Readable> Iterator for ProgramHeaderIter<'a, T> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        let program = self.nth(self.current);
        self.current += 1;
        program
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.elf.get_program(n)
    }
}

impl<'a, T: Readable> Elf<'a, T> {
    pub fn header(&self) -> &ElfHeader {
        &self.header
    }

    /// Returns an iterator over the sections in the elf
    fn get_sections(&'a self) -> SectionHeaderIter<'a, T> {
        SectionHeaderIter {
            elf: self,
            current: 0,
        }
    }

    /// Returns an iterator over the program headers in the elf
    fn get_programs(&'a self) -> ProgramHeaderIter<'a, T> {
        ProgramHeaderIter {
            elf: self,
            current: 0,
        }
    }

    /// Returns the offset of the program header at index `n` starting from the beginning of the file
    #[inline(always)]
    fn get_program_offset(&self, n: usize) -> Option<usize> {
        if n >= self.header.program_headers_table_entries_number as usize {
            return None;
        }

        let offset = self.header.program_headers_table_offset as usize
            + (n * self.header.program_headers_table_entry_size as usize);

        Some(offset)
    }

    /// Returns the program header at index `n`
    #[inline(always)]
    fn get_program(&'a self, n: usize) -> Option<ProgramHeader> {
        let offset = self.get_program_offset(n)?;

        let mut program_bytes = [0u8; size_of::<ProgramHeader>()];
        self.reader.read(offset as isize, &mut program_bytes).ok()?;
        Some(unsafe { core::mem::transmute(program_bytes) })
    }

    #[inline(always)]
    fn get_section_offset(&self, n: usize) -> Option<usize> {
        if n >= self.header.section_table_entries as usize {
            return None;
        }

        let offset = self.header.section_header_table_offset as usize
            + (n * self.header.section_table_entry_size as usize);

        Some(offset)
    }

    #[inline(always)]
    pub fn get_section(&self, n: usize) -> Option<SectionHeader> {
        let offset = self.get_section_offset(n)?;

        let mut section_bytes = [0u8; size_of::<SectionHeader>()];
        self.reader.read(offset as isize, &mut section_bytes).ok()?;
        Some(unsafe { core::mem::transmute(section_bytes) })
    }

    #[inline(always)]
    pub fn section_names_table(&self) -> Option<&SectionHeader> {
        self.names_table
            .call_once(|| self.get_section(self.header.sections_names_section_offset as usize))
            .as_ref()
    }

    pub fn section_names_table_index(&self, name_index: u32) -> Option<String> {
        if name_index == 0 {
            return None;
        }

        let name_table = self.section_names_table().unwrap();
        let section_offset = name_table.offset;
        let name_offset = section_offset + name_index as usize;

        let mut name = Vec::new();
        let mut c = [0u8];
        while let Ok(amount) = self
            .reader
            .read((name_offset + name.len()) as isize, &mut c)
        {
            if amount != 1 || c[0] == 0 {
                break;
            }
            name.push(c[0]);
        }

        String::from_utf8(name).ok()
    }

    #[inline]
    pub fn string_table(&self) -> Option<&SectionHeader> {
        self.strings_table
            .call_once(|| {
                self.get_sections().find(|section| {
                    self.section_names_table_index(section.name_index)
                        .is_some_and(|name| name == ".strtab")
                })
            })
            .as_ref()
    }

    pub fn string_table_index(&self, name_index: u32) -> Option<String> {
        if name_index == 0 {
            return None;
        }

        let str_table = self.string_table().unwrap();
        let section_offset = str_table.offset;
        let str_offset = section_offset + name_index as usize;

        let mut c = [0u8];
        let mut str = Vec::new();
        while let Ok(amount) = self.reader.read((str_offset + str.len()) as isize, &mut c) {
            if amount != 1 || c[0] == 0 {
                break;
            }
            str.push(c[0]);
        }
        String::from_utf8(str).ok()
    }

    #[inline]
    pub fn symtable(&self) -> Option<&[Sym]> {
        let func = || {
            self.get_sections()
                .find(|section| section.section_type == 2)
                .map(|section| {
                    debug_assert_eq!(section.entry_size, size_of::<Sym>());
                    let symtable_offset = section.offset;
                    let mut bytes = vec![0u8; section.size as usize];

                    self.reader
                        .read_exact(symtable_offset as isize, &mut bytes)
                        .ok()?;

                    let symtable: Vec<Sym> = unsafe {
                        let (ptr, len, cap) = bytes.into_raw_parts();
                        Vec::from_raw_parts(
                            ptr as *mut Sym,
                            len / section.entry_size,
                            cap / section.entry_size,
                        )
                    };
                    Some(symtable)
                })
                .flatten()
        };

        self.symbols.call_once(func).as_deref()
    }

    pub fn sym_from_value_range(&self, value: VirtAddr) -> Option<Sym> {
        for sym in self.symtable()? {
            if sym.value <= value && (sym.value + sym.size as usize) >= value {
                return Some(*sym);
            }
        }

        None
    }

    /// creates an elf from a u8 ptr that lives as long as `bytes`
    pub fn new(reader: &'a T) -> Result<Self, ElfError> {
        let mut header_bytes = [0u8; size_of::<ElfHeader>()];
        reader
            .read_exact(0, &mut header_bytes)
            .map_err(|_| ElfError::NotAnElf)?;
        let header: ElfHeader = unsafe { core::mem::transmute(header_bytes) };
        if !header.verify() {
            return Err(ElfError::NotAnElf);
        }

        header.supported()?;

        assert_eq!(
            size_of::<SectionHeader>(),
            header.section_table_entry_size as usize
        );

        assert_eq!(
            size_of::<ProgramHeader>(),
            header.program_headers_table_entry_size as usize
        );

        Ok(Self {
            header,
            names_table: Once::new(),
            strings_table: Once::new(),
            symbols: Once::new(),
            reader,
        })
    }

    /// loads an executable ELF, maps, and copies it to `page_table`.
    /// returns the program break on success.
    pub fn load_exec(&self, page_table: &mut PageTable) -> Result<VirtAddr, ElfError> {
        if self.header.kind != ElfType::EXE {
            return Err(ElfError::NotAnExecutable);
        }

        let mut program_break = 0;
        let mut buf = [0u8; PAGE_SIZE];

        for header in self.get_programs() {
            if header.ptype != ProgramType::LOAD {
                continue;
            }

            let mut entry_flags = EntryFlags::PRESENT | EntryFlags::USER_ACCESSIBLE;

            if header.flags.contains(ProgramFlags::READ) {
                entry_flags |= EntryFlags::empty();
            }

            if header.flags.contains(ProgramFlags::WRITE) {
                entry_flags |= EntryFlags::WRITABLE;
            }

            if header.flags.contains(ProgramFlags::EXEC) {
                entry_flags |= EntryFlags::WRITABLE;
            }

            let start_page = Page::containing_address(header.vaddr);
            let end_page = Page::containing_address(header.vaddr + header.memz + PAGE_SIZE);

            unsafe {
                for page in Page::iter_pages(start_page, end_page) {
                    let frame = frame_allocator::allocate_frame().ok_or(ElfError::MapToError)?;

                    page_table.map_to(page, frame, entry_flags)?;

                    let slice = slice::from_raw_parts_mut(frame.virt_addr() as *mut u8, PAGE_SIZE);
                    slice.fill(0x0);
                }

                let mut file_offset = header.offset;
                let mut size = header.filez;

                while let Ok(amount) = self.reader.read(file_offset as isize, &mut buf) {
                    if amount == 0 {
                        break;
                    }

                    let count = amount.min(size);
                    let buf = &buf[..count];

                    copy_to_userspace(
                        page_table,
                        header.vaddr + (file_offset - header.offset),
                        &buf,
                    );

                    size -= count;
                    if size == 0 {
                        break;
                    }

                    file_offset += count;
                }
            }
            program_break = header.vaddr + header.memz + PAGE_SIZE;
        }
        Ok(program_break)
    }
}
