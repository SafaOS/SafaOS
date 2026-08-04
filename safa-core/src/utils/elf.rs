use core::fmt::Display;

use alloc::vec;
use alloc::{string::String, vec::Vec};
use bitflags::bitflags;
use macros::display_consts;
use spin::once::Once;
use thiserror::Error;

use crate::drivers::vfs::FSError;
use crate::memory::AlignToPage;
use crate::memory::vmm::{Location, VMMAllocError, VMMMFlags, VirtualMemoryManager};
use crate::{PhysAddr, debug, error};
use crate::{
    VirtAddr,
    memory::{
        copy_to_pagetable,
        paging::{MapToError, PAGE_SIZE},
    },
};

use super::io::Readable;
use safa_abi::errors::{ErrorStatus, IntoErr};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ElfType(u16);
#[display_consts]
impl ElfType {
    pub const RELOC: ElfType = Self(1);
    pub const EXE: ElfType = Self(2);
    pub const DYN: ElfType = Self(3);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ElfInstrSet(u16);

#[display_consts]
impl ElfInstrSet {
    const AMD64: Self = Self(0x3E);
    const AARCH64: Self = Self(0xB7);
}

use cfg_if::cfg_if;
const SUPPORTED_INSTRUCTION_SETS: &[ElfInstrSet] = {
    cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            &[ElfInstrSet::AMD64]
        } else if #[cfg(target_arch = "aarch64")] {
            &[ElfInstrSet::AARCH64]
        } else {
            &[]
        }
    }
};

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
    pub instruction_set: ElfInstrSet,
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
pub struct TLSInfo {
    pub addr: VirtAddr,
    pub memsize: usize,
    pub filesize: usize,
    pub alignment: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ProgramHeaderInfo {
    pub addr: VirtAddr,
    pub ent_size: usize,
    pub ent_count: usize,
}

#[derive(Debug, Clone)]
pub struct ElfInfo {
    pub entry: VirtAddr,
    pub elf_end: VirtAddr,
    pub master_tls: Option<TLSInfo>,
    pub program_header: Option<ProgramHeaderInfo>,
    pub program_interp: Option<String>,
}

#[derive(Debug, Clone, Copy, Error)]
pub enum ElfOrFSError {
    #[error("Elf: {0}")]
    ElfError(#[from] ElfError),
    #[error("FS: {0}")]
    FSError(#[from] FSError),
}
#[derive(Debug, Clone, Copy, Error)]
pub enum ElfError {
    UnsupportedClass,
    UnsupportedEndianness,
    UnsupportedKind,
    UnsupportedInstructionSet,
    NotAnElf,
    NotAnExecutable,
    MapToError,
    Corrupted,
}

impl Display for ElfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl IntoErr for ElfError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::Corrupted => ErrorStatus::Corrupted,
            Self::NotAnExecutable | Self::NotAnElf => ErrorStatus::NotExecutable,
            Self::MapToError => ErrorStatus::MMapError,
            Self::UnsupportedKind
            | Self::UnsupportedInstructionSet
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
        } else if ![ElfType::EXE, ElfType::RELOC, ElfType::DYN].contains(&self.kind) {
            Err(ElfError::UnsupportedKind)
        } else if !SUPPORTED_INSTRUCTION_SETS.contains(&self.instruction_set) {
            Err(ElfError::UnsupportedInstructionSet)
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
    pub const INTERP: Self = Self(3);
    pub const PHDR: Self = Self(6);
    pub const TLS: Self = Self(7);
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
    pub vaddr: VirtAddr,
    pub paddr: PhysAddr,
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
impl<'a, T: Readable> Clone for ProgramHeaderIter<'a, T> {
    fn clone(&self) -> Self {
        Self {
            current: self.current,
            elf: self.elf,
        }
    }
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
    /// # Returns
    /// - Ok([`ElfInfo`]) if successful, info include master TLS, program interpreter, PT_PHDR...
    /// - Err([`ElfError::NotAnExecutable`]) if elf header isn't an [`ElfType::EXE`] or [`ElfType::DYN`].
    /// - Err([`ElfError::Corrupted`]) if elf header contains bad data such as invalid UTF8 in a PT_INTERP.
    pub fn load_exec(
        &self,
        vmm: &mut VirtualMemoryManager,
        mut load_at: VirtAddr,
    ) -> Result<ElfInfo, ElfError> {
        if self.header.kind != ElfType::EXE && self.header.kind != ElfType::DYN {
            return Err(ElfError::NotAnExecutable);
        }

        assert!(
            // NOT ET_EXE OR load at == 0
            self.header.kind != ElfType::EXE || load_at == VirtAddr::null(),
            "Cannot load an ET_EXE elf file at: {load_at:?}, ET_EXE elf can only be loaded at base 0"
        );

        if self.header.kind == ElfType::DYN && load_at == VirtAddr::null() {
            // TODO: Ensures nothing is loaded at NULL but I'm not quite sure if that is necessary.
            load_at = VirtAddr::from(0x1000);
        }

        let mut program_break = VirtAddr::null();

        let mut master_tls = None;
        let mut phdr = None;
        let mut interp = None;

        let mut buf = [0u8; PAGE_SIZE];

        let headers = self.get_programs().filter(|header| {
            header.ptype == ProgramType::LOAD
                || header.ptype == ProgramType::TLS
                || header.ptype == ProgramType::PHDR
                || header.ptype == ProgramType::INTERP
        });

        for header in headers {
            let vaddr = header.vaddr + load_at;

            let size_in_mem = header.memz;
            let alignment_in_mem = header.align;

            let start_addr = vaddr.to_previous_page();
            let end_addr = (vaddr + size_in_mem).to_next_page();
            let map_size = end_addr - start_addr;

            if vaddr + size_in_mem > program_break {
                program_break = vaddr + size_in_mem;
            }

            if header.ptype == ProgramType::INTERP {
                let mut bytes = Vec::new();

                let mut offset = header.offset;
                let mut size = header.filez;
                if size == 0 {
                    continue;
                }

                while let Ok(amount) = self.reader.read(offset as isize, &mut buf) {
                    if amount == 0 {
                        break;
                    }

                    let count = amount.min(size);
                    bytes.extend_from_slice(&buf[..count]);

                    offset += count;
                    size -= count;
                }

                let string = String::from_utf8(bytes).map_err(|e| {
                    error!("Invalid UTF-8 bytes in PT_INTERP {:?} ('{}'...), header: {header:#?}, elf_header: {:#?}", e.as_bytes(), String::from_utf8_lossy(&e.as_bytes()[..e.utf8_error().valid_up_to()]), self.header());
                    ElfError::Corrupted
                })?;

                debug!("{header:#?} => PT_INTERP {string}");
                interp = Some(string);
                continue;
            }

            let mut alloc_flags = VMMMFlags::USER_ACCESSIBLE | VMMMFlags::ZEROED;
            if header.flags.contains(ProgramFlags::READ) {
                alloc_flags |= VMMMFlags::empty();
            }

            if header.flags.contains(ProgramFlags::WRITE) {
                alloc_flags |= VMMMFlags::WRITABLE;
            }

            if header.flags.contains(ProgramFlags::EXEC) {
                alloc_flags |= VMMMFlags::EXECUTABLE;
            }

            fn map_frag(
                vmm: &mut VirtualMemoryManager,
                start_addr: VirtAddr,
                map_size: usize,
                alloc_flags: VMMMFlags,
            ) -> Result<(), ElfError> {
                // FIXME: Hacks to deal with fragmentation.
                match vmm.map_new(
                    &"elf.load",
                    Some(Location::Fixed(start_addr)),
                    map_size,
                    alloc_flags,
                    crate::memory::vmm::VMMAllocMode::Normal,
                ) {
                    Ok(_) => Ok(()),
                    Err(VMMAllocError::Used) => Ok(()),
                    Err(VMMAllocError::UsedBy {
                        at: r_at,
                        size: r_size,
                        flags: r_flags,
                    }) => {
                        // We want to map after the region within it.
                        if r_at <= start_addr {
                            let off_from = start_addr - r_at;
                            let size_left = r_size - off_from;
                            if map_size > size_left {
                                map_frag(
                                    vmm,
                                    start_addr + size_left,
                                    map_size - size_left,
                                    alloc_flags,
                                )?;
                            }
                        } else {
                            // We want to map before the region and beyond.
                            let can_map = r_at - start_addr;
                            map_frag(vmm, start_addr, can_map, alloc_flags)?;

                            let map_left = map_size - can_map;
                            if map_left > r_size {
                                map_frag(vmm, r_at + r_size, map_left - r_size, alloc_flags)?;
                            }
                        }

                        // Update flags if we have more prems.
                        let mut flags_or = VMMMFlags::empty();
                        if !r_flags.contains(VMMMFlags::WRITABLE)
                            && alloc_flags.contains(VMMMFlags::WRITABLE)
                        {
                            flags_or |= VMMMFlags::WRITABLE;
                        }

                        if !r_flags.contains(VMMMFlags::EXECUTABLE)
                            && alloc_flags.contains(VMMMFlags::EXECUTABLE)
                        {
                            flags_or |= VMMMFlags::EXECUTABLE;
                        }

                        if !flags_or.is_empty() {
                            let _ = vmm.set_page_flags(r_at, r_flags | flags_or);
                        }
                        Ok(())
                    }
                    Err(VMMAllocError::OutOfMemory) => {
                        return Err(ElfError::MapToError);
                    }
                    Err(VMMAllocError::InvalidSize | VMMAllocError::OutOfRange) => {
                        return Err(ElfError::Corrupted);
                    }
                }
            }

            map_frag(vmm, start_addr, map_size, alloc_flags)?;
            let page_table = unsafe { vmm.table_mut() };

            let mut file_offset = header.offset;
            let mut size = header.filez;

            while let Ok(amount) = self.reader.read(file_offset as isize, &mut buf) {
                if amount == 0 {
                    break;
                }

                let count = amount.min(size);
                let buf = &buf[..count];

                copy_to_pagetable(page_table, vaddr + (file_offset - header.offset), &buf);

                size -= count;
                if size == 0 {
                    break;
                }

                file_offset += count;
            }

            if header.ptype == ProgramType::TLS {
                let alignment_in_mem = alignment_in_mem.next_multiple_of(8);

                master_tls = Some(TLSInfo {
                    addr: vaddr,
                    memsize: size_in_mem,
                    filesize: header.filez,
                    alignment: alignment_in_mem,
                });
            }

            if header.ptype == ProgramType::PHDR {
                phdr = Some(ProgramHeaderInfo {
                    addr: vaddr,
                    ent_size: self.header().program_headers_table_entry_size as usize,
                    ent_count: self.header().program_headers_table_entries_number as usize,
                });
            }
        }
        Ok(ElfInfo {
            entry: self.header.entry_point,
            elf_end: program_break,
            master_tls,
            program_header: phdr,
            program_interp: interp,
        })
    }
}
