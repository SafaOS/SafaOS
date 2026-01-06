use core::{
    cell::UnsafeCell,
    fmt::Display,
    mem::MaybeUninit,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU16, Ordering},
};

use alloc::fmt;

use crate::{
    VirtAddr,
    arch::registers::ArchCpuID,
    debug,
    limine::MP_RESPONSE,
    memory::vmm::{self, Location, VMMMFlags},
};

/// Initializes the memory for SMP, should be called by [`crate::memory::init`]
pub fn init_memory(vmm: &mut vmm::VirtualMemoryManager) {
    let len = MP_RESPONSE.cpus().len();
    let size = (len - 1) * section_size();
    if size == 0 {
        return;
    }

    let start = section_end();

    vmm.map_new(
        &".percpu.ap",
        Some(Location::Fixed(start)),
        size,
        VMMMFlags::ZEROED | VMMMFlags::WRITEABLE,
        vmm::VMMAllocMode::Normal,
    )
    .expect("Failed to allocate space for percpus");
}

pub type PerCpuInitializer = fn(&'static CpuLocal);

unsafe extern "C" {
    static section_per_cpu_begin: u8;
    static section_per_cpu_end: u8;
    static section_per_cpu_init_begin: u8;
    static section_per_cpu_init_end: u8;
}

#[inline]
fn initializers_start() -> VirtAddr {
    unsafe { VirtAddr::from((&section_per_cpu_init_begin as *const u8) as usize) }
}

#[inline]
fn initializers_end() -> VirtAddr {
    unsafe { VirtAddr::from((&section_per_cpu_init_end as *const u8) as usize) }
}

#[inline]
fn per_cpu_initializers() -> &'static [PerCpuInitializer] {
    unsafe {
        let start = initializers_start();
        let end = initializers_end();
        core::slice::from_raw_parts(
            start.into_ptr::<PerCpuInitializer>(),
            (end - start) / core::mem::size_of::<PerCpuInitializer>(),
        )
    }
}

#[inline(always)]
fn section_start() -> VirtAddr {
    unsafe { VirtAddr::from((&section_per_cpu_begin as *const u8) as usize) }
}

#[inline(always)]
fn section_end() -> VirtAddr {
    unsafe { VirtAddr::from((&section_per_cpu_end as *const u8) as usize) }
}

#[inline(always)]
fn section_size() -> usize {
    section_end() - section_start()
}

/// A Special assigned ID to each CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuID(u16);

impl CpuID {
    /// Creates a new [`CpuID`] from a u16.
    ///
    /// Returns `None` if the u16 is greater than or equal to the maximum number of CPUs.
    pub fn from_u16(id: u16) -> Option<Self> {
        let max = NEXT_CPU_ID.load(Ordering::Relaxed);

        if id < max { Some(Self(id)) } else { None }
    }
}

impl Display for CpuID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CPU #{}", self.0)
    }
}

/// An iterator over all [`CpuLocalStorage`]s for each CPU.
#[derive(Debug, Clone, Copy)]
pub struct CpuLocalStoragesIter {
    current_id: CpuID,
    max: u16,
}

impl CpuLocalStoragesIter {
    /// Creates a new iterator over all [`CpuLocalStorage`]s for each CPU.
    ///
    /// `max` - 1 must be a valid CpuID.
    pub const unsafe fn new(max: u16) -> Self {
        Self {
            current_id: CpuID(0),
            max,
        }
    }

    #[inline]
    #[allow(unused)]
    pub fn len_hint(&self) -> usize {
        self.max as usize
    }
}

impl Iterator for CpuLocalStoragesIter {
    type Item = &'static CpuLocal;
    fn next(&mut self) -> Option<Self::Item> {
        if self.max <= self.current_id.0 {
            return None;
        }

        let current = CpuLocal::get_for(self.current_id);
        self.current_id.0 += 1;
        if !current.online.load(Ordering::Relaxed) {
            return self.next();
        }
        Some(current)
    }
}

/// The big main [`PerCpuStorage`], stored at the very beginning.
#[derive(Debug)]
#[repr(C)]
pub struct CpuLocal {
    ptr_to_self: NonNull<CpuLocal>,
    /// The identifier assigned to each CPU.
    pub cpu_id: CpuID,
    /// The architecture-specific identifier assigned to each CPU.
    pub cpu_arch_id: ArchCpuID,
    pub online: AtomicBool,
}

impl CpuLocal {
    #[inline]
    pub fn get() -> &'static Self {
        unsafe { &*crate::arch::smp::current_local_ptr() }
    }

    #[inline]
    pub fn get_for(id: CpuID) -> &'static Self {
        let off = section_size() * id.0 as usize;
        let addr = section_start() + off;
        unsafe { (*addr.into_ptr::<PerCpuStorage<CpuLocal>>()).borrow_this() }
    }

    #[inline]
    pub fn get_bsp() -> &'static Self {
        unsafe { (*BASE_CPU_LOCAL.data.get()).assume_init_ref() }
    }

    #[inline]
    pub fn get_all() -> CpuLocalStoragesIter {
        unsafe { CpuLocalStoragesIter::new(NEXT_CPU_ID.load(Ordering::Relaxed)) }
    }
}
unsafe impl Send for CpuLocal {}
unsafe impl Sync for CpuLocal {}

#[repr(C)]
/// Represents a per-CPU storage for a static.
///
/// It is initialized once per CPU given a specific constructor function, it is stored in a special section that is copied and initialized at boot.
///
/// NOTE: It is important to avoid using allocations or anything blocking.
///
/// Before initialization, the data is zeroed.
pub struct PerCpuStorage<T: 'static> {
    initialized: UnsafeCell<bool>,
    data: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send> Send for PerCpuStorage<T> {}
unsafe impl<T: Sync> Sync for PerCpuStorage<T> {}

impl<T: 'static> PerCpuStorage<T> {
    #[inline(always)]
    pub fn init(&mut self, v: T) {
        let initialized = self.initialized.get();
        let data = self.data.get();
        if unsafe { initialized.read_volatile() } {
            return;
        }

        unsafe { data.write_volatile(MaybeUninit::new(v)) };
        unsafe { initialized.write_volatile(true) };
    }

    /// Creates a new [`PerCpuStorage`] instance, with zeroed data.
    pub const fn new_zeroed() -> Self {
        Self {
            initialized: UnsafeCell::new(false),
            data: UnsafeCell::new(MaybeUninit::zeroed()),
        }
    }

    #[allow(unused)]
    /// Creates a new [`PerCpuStorage`] instance with valid data, but it is not marked as initialized.
    pub const fn new_uninit(placeholder: T) -> Self {
        Self {
            initialized: UnsafeCell::new(false),
            data: UnsafeCell::new(MaybeUninit::new(placeholder)),
        }
    }

    #[allow(unused)]
    /// Same as [`new_uninit`] but the data is marked as initialized.
    pub const fn new_const(constant: T) -> Self {
        Self {
            initialized: UnsafeCell::new(true),
            data: UnsafeCell::new(MaybeUninit::new(constant)),
        }
    }

    #[inline(always)]
    pub fn get_from_ptr(&self, cpu: &'static CpuLocal) -> *mut Self {
        let offset = VirtAddr::from_ptr(self) - section_start();

        unsafe {
            (cpu as *const _ as *const u8)
                .byte_sub(core::mem::offset_of!(PerCpuStorage<CpuLocal>, data))
                .byte_add(offset)
                .cast::<Self>()
                .cast_mut()
        }
    }

    /// Given a CPU base, returns a reference to the [`PerCpuStorage`] instance for that CPU.
    #[inline(always)]
    pub unsafe fn get_from(&self, cpu: &'static CpuLocal) -> &Self {
        unsafe { &*self.get_from_ptr(cpu) }
    }

    #[inline]
    /// Returns a reference to the [`PerCpuStorage`] instance for the current CPU, by default it is a reference to the BSP's instance.
    pub fn get_curr(&self) -> &Self {
        unsafe { self.get_from(CpuLocal::get()) }
    }

    /// Same as [`Self::borrow`], but borrows from this reference not necessarily from the current CPU.
    #[inline(always)]
    pub fn borrow_this(&self) -> &T {
        debug_assert!(
            unsafe { *self.initialized.get() },
            "Attempt to borrow uninitialized data, {:?}",
            self.initialized.get()
        );
        unsafe { self.borrow_this_uninit() }
    }

    /// Same as [`Self::borrow_uninit`], but borrows from this reference not necessarily from the current CPU.
    #[inline(always)]
    pub const unsafe fn borrow_this_uninit(&self) -> &T {
        unsafe { (*self.data.get()).assume_init_ref() }
    }

    /// Same as [`Self::maybe_borrow`], but borrows from this reference not necessarily from the current CPU.
    #[inline(always)]
    pub fn maybe_borrow_this(&self) -> Option<&T> {
        unsafe {
            self.initialized
                .get()
                .read()
                .then(|| (*self.data.get()).assume_init_ref())
        }
    }

    #[inline]
    /// Borrows the data stored in the [`PerCpuStorage`], for the current CPU.
    ///
    /// Safety: Must be called after the CPU has been initialized, panicks if not in debug mode.
    pub fn borrow(&self) -> &T {
        self.get_curr().borrow_this()
    }

    #[inline]
    /// Same as [`Self::borrow`], but borrows for the given CPU.
    pub fn borrow_for(&self, cpu: &'static CpuLocal) -> &T {
        unsafe { self.get_from(cpu).borrow_this() }
    }

    #[inline]
    /// Borrows the data stored in the [`PerCpuStorage`] if it has been initialized, for the current CPU.
    ///
    /// Returns `None` if the data has not been initialized yet.
    ///
    /// Purpose: Allow access to urgent data without panicking when for example executing an interrupt, you may use this to access the scheduler if it wasn't initialized.
    pub fn maybe_borrow(&self) -> Option<&T> {
        self.get_curr().maybe_borrow_this()
    }

    #[allow(unused)]
    /// Borrows maybe uninitialized data without panicking, if data has not been initialized yet, for the current CPU.
    ///
    /// Purpose: Allow access to data created with [`PerCpuStorage::new_uninit`], before CPU initialization.
    pub unsafe fn borrow_uninit(&'static self) -> &'static T {
        unsafe { self.get_curr().borrow_this_uninit() }
    }
}

impl<T> Deref for PerCpuStorage<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

/// The next CPU ID to be assigned.
static NEXT_CPU_ID: AtomicU16 = AtomicU16::new(1);

/// Defines a per-CPU static variable.
///
/// # Syntax:
/// define! {
///     static name: ty = |base| ...; $(placeholder = ...;)?
/// }
///
/// Where name is the name of the variable, ty is the type of the variable, base is a reference to a [`BASE_CPU_LOCAL`], placeholder is the uninitialized data placeholder, if not given, the data will be zero-initialized.
///
/// You can access placeholders with [`PerCpuStorage::borrow_uninit`].
///
/// You can also provide attributes and visibility modifiers.
#[macro_export]
macro_rules! define {
    { $(#[$attr:meta])* $vis:vis static $name:ident: $ty:ty = |$base:ident| $f:expr; $(placeholder = $placeholder:expr;)? $(const = $const:expr;)? $section:literal,$initializer_section:literal } => {
        #[allow(unreachable_code)]
        #[allow(unused_labels)]
        $(#[$attr])*
        #[used]
        #[unsafe(link_section = $section)]
        $vis static $name: $crate::percpu::PerCpuStorage<$ty> = 'blck: {
            $(break 'blck $crate::percpu::PerCpuStorage::new_uninit($placeholder);)?
            $(break 'blck $crate::percpu::PerCpuStorage::new_const($const);)?
            $crate::percpu::PerCpuStorage::new_zeroed()
        };

        const _: () = {
            #[used]
            #[allow(unused_labels)]
            #[allow(unreachable_code)]
            #[unsafe(link_section = $initializer_section)]
            static INITIALIZER: $crate::percpu::PerCpuInitializer = {
                 |base| {
                    let $base = base;
                    let i = 'block: {
                        $(_ = $base;const CONSTANT: $ty = $const; break 'block CONSTANT;)?
                        $f
                    };
                    unsafe { (&mut *$name.get_from_ptr(base)).init(i) };
                }
            };
        };
    };
    { $(#[$attr:meta])* $vis:vis  static $name:ident: $ty:ty = |$base: ident| $f:expr; $(placeholder = $placeholder:expr;)? } => {
        $crate::percpu::define! {
            $(#[$attr])*
            $vis static $name: $ty = |$base| $f; $(placeholder = $placeholder;)? ".percpu", ".percpu.initializers"
        }
    };

    { $(#[$attr:meta])* $vis:vis static $name:ident: $ty:ty = const { $f:expr }; } => {
        $crate::percpu::define! {
            $(#[$attr])*
            $vis static $name: $ty = |base| {
                _ = base;
                const { $f }
            }; const = const { $f }; ".percpu", ".percpu.initializers"
        }
    };

    { $(#[$attr:meta])* $vis:vis static $name:ident: $ty:ty = $f:expr; $(placeholder = $placeholder:expr;)? } => {
        $crate::percpu::define! {
            $(#[$attr])*
            $vis static $name: $ty = |base| {
                _ = base;
                $f
            }; $(placeholder = $placeholder;)? ".percpu", ".percpu.initializers"
        }
    };

}
pub use crate::define;

define! {
    /// Base CPU-local storage.
    ///
    /// contains identifiers and such.
    pub static BASE_CPU_LOCAL: CpuLocal = |this| {
        CpuLocal {
            ptr_to_self: NonNull::from_ref(this),
            cpu_id: this.cpu_id,
            cpu_arch_id: this.cpu_arch_id,
            online: AtomicBool::new(false),
        }
    }; ".percpu.base", ".percpu.initializers.base"
}

/// Initialize the BSP's CPU-local storage for boot usage before SMP initialization, returning a reference to the base.
pub fn init_bsp_first() -> &'static CpuLocal {
    let cpu_local = CpuLocal::get_bsp();
    per_cpu_initializers()[0](cpu_local);

    assert!(
        unsafe { BASE_CPU_LOCAL.initialized.get().read_volatile() },
        "base CPULocal wasn't initialized..."
    );

    cpu_local
}
/// Initialize the BSP's CPU-local storage, returning a reference to the base.
pub fn init_bsp_all() -> &'static CpuLocal {
    let cpu_local = CpuLocal::get_bsp();
    for init in per_cpu_initializers() {
        (init)(cpu_local)
    }

    cpu_local.online.store(true, Ordering::SeqCst);
    cpu_local
}

/// Allocates and initializes a new CPU-local storage, returning a reference to the base.
pub fn allocate_next(arch_id: ArchCpuID) -> &'static CpuLocal {
    let id = NEXT_CPU_ID.fetch_add(1, Ordering::Relaxed);
    // Allocate memory
    let index = id as usize;

    let size = section_size();
    let base = section_start() + (size * index);

    debug!("Allocating CPU Local at: {base:?} with size: {size:#x}");
    let cpu_local_container = unsafe { &mut *base.into_ptr::<PerCpuStorage<CpuLocal>>() };
    let cpu_local = unsafe { (*cpu_local_container.data.get()).assume_init_mut() };
    cpu_local.cpu_id = CpuID(id as u16);
    cpu_local.cpu_arch_id = arch_id;

    for init in per_cpu_initializers() {
        (init)(cpu_local)
    }

    cpu_local
}
