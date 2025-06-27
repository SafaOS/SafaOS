use crate::utils::alloc::PageString;

#[derive(Debug)]
pub(super) struct GenericRodFSFile {
    name: &'static str,
    pub id: usize,
    data: Option<PageString>,
    /// if true the data won't be de-allocated when the file is closed
    is_static: bool,
    fetch: fn(&mut Self) -> Option<PageString>,
}

impl GenericRodFSFile {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub const fn new(
        name: &'static str,
        id: usize,
        fetch: fn(&mut Self) -> Option<PageString>,
    ) -> Self {
        Self {
            name,
            id,
            data: None,
            is_static: false,
            fetch,
        }
    }

    pub const fn new_static(
        name: &'static str,
        id: usize,
        fetch: fn(&mut Self) -> Option<PageString>,
    ) -> Self {
        Self {
            name,
            id,
            data: None,
            is_static: true,
            fetch,
        }
    }

    pub(super) fn get_data(&mut self) -> &str {
        if self.data.is_none() {
            self.refresh();
        }

        self.data.as_ref().unwrap().as_str()
    }

    pub(super) fn close(&mut self) {
        if !self.is_static {
            self.data = None;
        }
    }

    fn refresh(&mut self) {
        let fetch = self.fetch;
        self.data = fetch(self);
    }
}
