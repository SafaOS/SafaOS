/// A PCI ExtendedCaptability
pub trait ExtendedCaptability: Sized {
    fn id() -> u8;
    #[allow(unused)]
    fn header(&self) -> &GenericCaptability;
    /// Shouldn't be manually implemented
    unsafe fn from_dwords(dwords: *mut u32) -> Self {
        let slice = core::slice::from_raw_parts(dwords, size_of::<Self>() / 4);
        unsafe { core::mem::transmute_copy(&slice) }
    }
}

/// A generic captabilitiy, all PCI extended captabilites share this as a header
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct GenericCaptability {
    id: u8,
    next_off: u8,
}

pub struct CaptabilitiesIter {
    base_ptr: *const (),
    current: *const GenericCaptability,
}

impl CaptabilitiesIter {
    pub fn new(base_ptr: *const (), cap_off: u8) -> Self {
        let current = unsafe { base_ptr.byte_add(cap_off as usize) as *const GenericCaptability };
        Self { base_ptr, current }
    }

    pub fn empty() -> Self {
        Self {
            base_ptr: core::ptr::null(),
            current: core::ptr::null(),
        }
    }

    /// Find the next captabilitiy with the id `id`
    pub fn find_next(&mut self, id: u8) -> Option<*const GenericCaptability> {
        while let Some(cap_ptr) = self.next() {
            let cap = unsafe { *cap_ptr };
            if cap.id == id {
                return Some(cap_ptr);
            }
        }

        None
    }

    /// Find a captability with the `T::id(` id and then casts it to a pointer of T
    pub fn find_next_cast<T: ExtendedCaptability>(&mut self) -> Option<*const T> {
        self.find_next(T::id()).map(|ptr| ptr.cast())
    }

    /// Find a captability with the `id` id and then casts it to a pointer of T, consuming self
    pub fn find_cast<T: ExtendedCaptability>(mut self) -> Option<*const T> {
        self.find_next_cast()
    }

    /// Find a captability with the `id` id and then transmutes into T correctly performing dword reads
    pub unsafe fn find_next_transmute<T: ExtendedCaptability>(&mut self) -> Option<T> {
        self.find_next_cast::<T>().map(|ptr| {
            let dword_ptr = ptr as *mut u32;
            unsafe { T::from_dwords(dword_ptr) }
        })
    }
}

impl Iterator for CaptabilitiesIter {
    type Item = *const GenericCaptability;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            return None;
        }

        let next_off = unsafe { (*self.current).next_off };
        let results = self.current;

        if next_off == 0 {
            self.current = core::ptr::null();
        } else {
            self.current =
                unsafe { self.base_ptr.byte_add(next_off as usize) as *const GenericCaptability };
        }
        Some(results)
    }
}
