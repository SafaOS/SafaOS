//! The kernel's boot initialization subsystem.
//!
//! You define a routine that runs at boot using
//! ```rs
//! oninit::define_routine! {
//!     unsafe fn XYZ = with DEP1,DEP2 || { .. };
//! }
//! ```
//!
//! or a static using:
//! ```rs
//! oninit::define! {
//!     unsafe static XYZ: Xyz = with DEP1,DEP2 || { .. };
//! }
//! ```
//!
//! where DEP1 and DEP2 are identifiers for other oninit routines/statics so they are initialized before XYZ.
//!
//! examples: [`crate::bootloader::INI_BOOTLOADER_MEMORY`], [`crate::mem::slab::INI_SLAB`], [`crate::mem::alloc::INI_ALLOC`].
use core::{
    cell::{SyncUnsafeCell, UnsafeCell},
    mem::MaybeUninit,
    ops::Deref,
};

use crate::misc::VirtAddr;

unsafe extern "C" {
    static section_oninit_f_begin: u8;
    static section_oninit_f_end: u8;
}

#[inline]
pub fn initializers_start() -> VirtAddr {
    unsafe { VirtAddr::from((&section_oninit_f_begin as *const u8) as usize) }
}

#[inline]
pub fn initializers_end() -> VirtAddr {
    unsafe { VirtAddr::from((&section_oninit_f_end as *const u8) as usize) }
}

#[inline]
fn oninit_initializers() -> &'static [OnInitRoutine] {
    unsafe {
        let start = initializers_start();
        let end = initializers_end();
        core::slice::from_raw_parts(
            start.into_ptr::<OnInitRoutine>(),
            (end - start) / core::mem::size_of::<OnInitRoutine>(),
        )
    }
}

pub trait OnInitDep: Send + Sync {
    fn as_routine(&self) -> &OnInitRoutine;
}

pub struct OnInitRoutine {
    ran: SyncUnsafeCell<bool>,
    deps: &'static [&'static dyn OnInitDep],
    func: unsafe fn(),
}

impl OnInitRoutine {
    pub const fn new(deps: &'static [&'static dyn OnInitDep], func: unsafe fn()) -> Self {
        Self {
            ran: SyncUnsafeCell::new(false),
            deps,
            func,
        }
    }

    /// Runs the oninit routine, if it has not already been run.
    /// Runs the dependencies first, then the function itself.
    ///
    /// # Safety: no synchronization promises
    pub unsafe fn run(&self) {
        unsafe {
            if *self.ran.get() {
                return;
            }
            for dep in self.deps {
                dep.as_routine().run();
            }
            (self.func)();
            *self.ran.get() = true;
        }
    }
}

impl OnInitDep for OnInitRoutine {
    fn as_routine(&self) -> &OnInitRoutine {
        self
    }
}

/// Represents an oninit storage for a static.
///
/// It is initialized once on startup given a specific constructor function, it is stored in a special section that is copied and initialized at boot.
///
/// NOTE: It is important to avoid using allocations or anything blocking.
///
/// Before initialization, the data is zeroed.
pub struct OnInitStorage<T: 'static> {
    data: UnsafeCell<MaybeUninit<T>>,
    routine: &'static OnInitRoutine,
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
    pub const unsafe fn new_zeroed(routine: &'static OnInitRoutine) -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::zeroed()),
            routine,
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

impl<T: Send + Sync> OnInitDep for OnInitStorage<T> {
    fn as_routine(&self) -> &OnInitRoutine {
        self.routine
    }
}

#[macro_export]
macro_rules! _defineroutine {
    { $(#[$attr:meta])* $vis:vis unsafe fn $name:ident = $(with $($dep:ident),*)? || $f:expr; } => {
        $(#[$attr])*
        #[allow(unreachable_code)]
        #[allow(unused_labels)]
        #[used]
        #[unsafe(link_section = ".oninit.initializers")]
        $vis static $name: $crate::oninit::OnInitRoutine = {
            $crate::oninit::OnInitRoutine::new(&[$($(&$dep,)*)?], || $f)
        };
    };

    { $( $(#[$attr:meta])* $vis:vis unsafe fn $name:ident = $(with $($dep:ident),*)? || $f:expr; )* } => {
        $(
            $crate::oninit::define_routine! { $(#[$attr])* $vis unsafe fn $name = $(with $($dep),*)? || $f; }
        )*
    }
}

/// Defines a static to be filled on boot initialization with a value for fast access later.
///
/// Must define all it's oninit dependencies
#[macro_export]
macro_rules! _defineoninit {
    { $(#[$attr:meta])* $vis:vis unsafe static $name:ident: $ty:ty = $(with $($dep:ident),*)? || $f:expr; } => {
        #[allow(unreachable_code)]
        #[allow(unused_labels)]
        $(#[$attr])*
        #[used]
        $vis static $name: $crate::oninit::OnInitStorage<$ty> = 'blck: {

            $crate::oninit::define_routine! {
                unsafe fn INITIALIZER = $(with $($dep),*)? || {
                    let i = 'block: {
                        $f
                    };
                    unsafe { $name.init(i) };
                };
            }

            unsafe { $crate::oninit::OnInitStorage::new_zeroed(&INITIALIZER) }
        };
    };
}

pub use _defineoninit as define;
pub use _defineroutine as define_routine;

/// Runs all oninit routines in the correct order, as defined by their dependencies.
///
/// # Safety: no synchronization promises should be ran at boot first.
pub unsafe fn init() {
    for init in oninit_initializers() {
        unsafe { init.run() };
    }
}
