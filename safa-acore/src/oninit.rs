use core::{cell::UnsafeCell, mem::MaybeUninit, ops::Deref};

use crate::misc::VirtAddr;

pub type OnInitInitializer = fn();

unsafe extern "C" {
    static section_oninit_f_begin: u8;
    static section_oninit_f_end: u8;
}

#[inline]
fn initializers_start() -> VirtAddr {
    unsafe { VirtAddr::from((&section_oninit_f_begin as *const u8) as usize) }
}

#[inline]
fn initializers_end() -> VirtAddr {
    unsafe { VirtAddr::from((&section_oninit_f_end as *const u8) as usize) }
}

#[inline]
fn oninit_initializers() -> &'static [OnInitInitializer] {
    unsafe {
        let start = initializers_start();
        let end = initializers_end();
        core::slice::from_raw_parts(
            start.into_ptr::<OnInitInitializer>(),
            (end - start) / core::mem::size_of::<OnInitInitializer>(),
        )
    }
}

#[repr(C)]
/// Represents an oninit storage for a static.
///
/// It is initialized once on startup given a specific constructor function, it is stored in a special section that is copied and initialized at boot.
///
/// NOTE: It is important to avoid using allocations or anything blocking.
///
/// Before initialization, the data is zeroed.
pub struct OnInitStorage<T: 'static> {
    data: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send> Send for OnInitStorage<T> {}
unsafe impl<T: Sync> Sync for OnInitStorage<T> {}

impl<T: 'static> OnInitStorage<T> {
    #[inline(always)]
    pub unsafe fn init(&self, v: T) {
        let data = self.data.get();
        unsafe { data.write_volatile(MaybeUninit::new(v)) };
    }

    /// Creates a new [`OninitStorage`] instance, with zeroed data.
    pub const unsafe fn new_zeroed() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::zeroed()),
        }
    }

    pub const fn borrow(&self) -> &T {
        unsafe { (&*self.data.get()).assume_init_ref() }
    }
}

impl<T> Deref for OnInitStorage<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

/// Defines a static to be filled on boot initialization with a value for fast access later.
///
/// Must not have any complex initialization functions, initialized before the scheduler and proper memory...
#[macro_export]
macro_rules! _defineoninit {
    { $(#[$attr:meta])* $vis:vis static $name:ident: $ty:ty = || $f:expr; $section:literal,$initializer_section:literal } => {
        #[allow(unreachable_code)]
        #[allow(unused_labels)]
        $(#[$attr])*
        #[used]
        #[unsafe(link_section = $section)]
        $vis static $name: $crate::oninit::OnInitStorage<$ty> = 'blck: {
            unsafe { $crate::oninit::OnInitStorage::new_zeroed() }
        };

        const _: () = {
            #[used]
            #[allow(unused_labels)]
            #[allow(unreachable_code)]
            #[unsafe(link_section = $initializer_section)]
            static INITIALIZER: $crate::oninit::OnInitInitializer = {
                 || {
                    let i = 'block: {
                        $f
                    };
                    unsafe { $name.init(i) };
                }
            };
        };
    };
    { $(#[$attr:meta])* $vis:vis  static $name:ident: $ty:ty = || $f:expr; } => {
        $crate::oninit::define! {
            $(#[$attr])*
            $vis static $name: $ty = || $f; ".oninit", ".oninit.initializers"
        }
    };
}

pub use _defineoninit as define;

pub unsafe fn init() {
    for init in oninit_initializers() {
        init();
    }
}
