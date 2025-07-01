use crate::{drivers::vfs::rodatafs::GenericRodFSFile, utils::alloc::PageString};

pub struct EVEJournal;

impl EVEJournal {
    pub const fn new() -> GenericRodFSFile {
        GenericRodFSFile::new("eve-journal", 0, Self::fetch)
    }

    pub fn fetch(_: &mut GenericRodFSFile) -> Option<PageString> {
        // FIXME: even tho this function returns an Option it is not respected and it'd panic if SERIAL_LOG is not initialized
        Some(
            crate::logging::SERIAL_LOG
                .read()
                .clone()
                .unwrap_or(PageString::new()),
        )
    }
}
