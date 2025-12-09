use core::{
    alloc::Layout,
    cell::SyncUnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::alloc::Global;

use crate::arch::smp::CPULocal;

/// A Global Storage for CPUS' Local storages
pub struct CPULocalsContainer {
    bsp_cpu_local: MaybeUninit<CPULocal>,
    cpu_locals_ptr: NonNull<[&'static CPULocal]>,
    smp_cpu_locals: NonNull<[CPULocal]>,
}

unsafe impl Send for CPULocalsContainer {}
unsafe impl Sync for CPULocalsContainer {}

impl CPULocalsContainer {
    pub const fn new() -> Self {
        Self {
            bsp_cpu_local: MaybeUninit::uninit(),
            cpu_locals_ptr: NonNull::from_ref(&[]),
            smp_cpu_locals: NonNull::from_ref(&[]),
        }
    }

    pub fn set_cpu(&mut self, index: usize, local: CPULocal) -> &mut CPULocal {
        if index == 0 {
            let ptr = self.bsp_cpu_local.write(local);
            ptr.on_allocated();
            ptr
        } else {
            let smp_index = index - 1;
            unsafe {
                let place = &mut self.smp_cpu_locals.as_mut()[smp_index];
                *place = local;
                place.on_allocated();
                place
            }
        }
    }

    pub fn reserve(&'static mut self, extra_cpus_count: usize) {
        use core::alloc::Allocator;
        let layout0 = Layout::from_size_align(
            size_of::<CPULocal>() * extra_cpus_count,
            align_of::<CPULocal>(),
        )
        .unwrap();
        let layout1 = Layout::from_size_align(
            size_of::<&'static CPULocal>() * (extra_cpus_count + 1),
            align_of::<&'static CPULocal>(),
        )
        .unwrap();

        let bytes_first = Global
            .allocate(layout0)
            .expect("Failed to allocate memory for CPU Local");
        let bytes_second = Global
            .allocate(layout1)
            .expect("Failed to allocate memory for CPU Local");

        self.smp_cpu_locals = NonNull::slice_from_raw_parts(
            bytes_first.cast::<CPULocal>(),
            bytes_first.len() / size_of::<CPULocal>(),
        );
        self.cpu_locals_ptr = NonNull::slice_from_raw_parts(
            bytes_second.cast::<&'static CPULocal>(),
            bytes_second.len() / size_of::<&'static CPULocal>(),
        );

        for i in 0..self.cpu_locals_ptr.len() {
            unsafe {
                if i == 0 {
                    self.cpu_locals_ptr.as_mut()[0] = self.bsp_cpu_local.assume_init_ref();
                } else {
                    self.cpu_locals_ptr.as_mut()[i] = &self.smp_cpu_locals.as_ref()[i - 1];
                }
            }
        }
    }
}

static CPU_LOCAL_CONTAINER: SyncUnsafeCell<CPULocalsContainer> =
    SyncUnsafeCell::new(CPULocalsContainer::new());
static NEXT_CPU_LOCAL_INDEX: AtomicUsize = AtomicUsize::new(0);

/// By default we can only hold one CPU Local storage and we don't allocate
///
/// but if SMP was detected this function has to be called with the amount of extra CPUs to reserve space for, once the allocator is initialized.
pub fn reserve_cpus(cpus_count: usize) {
    if cpus_count != 0 {
        unsafe { &mut *CPU_LOCAL_CONTAINER.get() }.reserve(cpus_count);
    }
}

/// Safely allocate a place for storing CPULocals
///
/// [`reserve_cpus`] must be called if you want to allocate more than 1 CPU, if you attempt to allocate to much CPU Locals, this will panic
pub fn allocate_cpu_local(local: CPULocal) -> &'static mut CPULocal {
    let index = NEXT_CPU_LOCAL_INDEX.fetch_add(1, Ordering::Relaxed);
    unsafe { &mut *CPU_LOCAL_CONTAINER.get() }.set_cpu(index, local)
}

/// Gets references to all allocated CPU Locals
pub fn get_all_cpu_locals() -> &'static [&'static CPULocal] {
    unsafe { (*CPU_LOCAL_CONTAINER.get()).cpu_locals_ptr.as_ref() }
}

impl CPULocal {
    pub fn get_all() -> &'static [&'static CPULocal] {
        get_all_cpu_locals()
    }
}
