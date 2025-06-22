use crate::{drivers::vfs::procfs::ProcFSFile, utils::alloc::PageString};

pub struct EVEJournal;

impl EVEJournal {
    pub fn new() -> ProcFSFile {
        ProcFSFile::new("eve-journal", 0, Self::fetch)
    }

    pub fn fetch(_: &mut ProcFSFile) -> Option<PageString> {
        crate::serial!("READING!\n");
        // FIXME: even tho this function returns an Option it is not respected and it'd panic if SERIAL_LOG is not initialized
        Some(
            crate::logging::SERIAL_LOG
                .read()
                .clone()
                .unwrap_or(PageString::new()),
        )
    }
}
