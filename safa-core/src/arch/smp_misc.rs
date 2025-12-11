use core::{alloc::Layout, mem::MaybeUninit, ptr::NonNull};

use alloc::alloc::Global;

use crate::{arch::smp::CPULocal, utils::locks::SpinLock};

/// A Global Storage for CPUS' Local storages
pub struct CPULocalsContainer {
    bsp_cpu_local: MaybeUninit<CPULocal>,
    bsp_cpu_local_ptr: MaybeUninit<&'static CPULocal>,
    cpu_locals_ptr: NonNull<[&'static CPULocal]>,
    smp_cpu_locals: NonNull<[CPULocal]>,
    allocated_len: usize,
}

unsafe impl Send for CPULocalsContainer {}
unsafe impl Sync for CPULocalsContainer {}

impl CPULocalsContainer {
    pub const fn new() -> Self {
        Self {
            bsp_cpu_local: MaybeUninit::uninit(),
            bsp_cpu_local_ptr: MaybeUninit::uninit(),
            cpu_locals_ptr: NonNull::from_ref(&[]),
            smp_cpu_locals: NonNull::from_ref(&[]),
            allocated_len: 0,
        }
    }

    fn set_cpu(&mut self, index: usize, local: CPULocal) -> NonNull<CPULocal> {
        if index == 0 {
            let ptr = self.bsp_cpu_local.write(local);
            ptr.on_allocated();
            let ptr = NonNull::from_mut(ptr);
            self.bsp_cpu_local_ptr = MaybeUninit::new(unsafe { ptr.as_ref() });
            ptr
        } else {
            let smp_index = index - 1;
            unsafe {
                let place = &mut self.smp_cpu_locals.as_mut()[smp_index];
                *place = local;
                place.on_allocated();
                NonNull::from_mut(place)
            }
        }
    }

    unsafe fn cpu_locals<'a>(&'a self) -> &'static [&'static CPULocal] {
        if self.allocated_len == 1 {
            core::hint::cold_path();
            unsafe { core::slice::from_ref(&*self.bsp_cpu_local_ptr.as_ptr()) }
        } else {
            unsafe { &self.cpu_locals_ptr.as_ref()[..self.allocated_len] }
        }
    }

    pub fn insert_next(&mut self, local: CPULocal) -> NonNull<CPULocal> {
        let index = self.allocated_len;
        let results = self.set_cpu(index, local);
        self.allocated_len += 1;
        results
    }

    pub fn reserve(&mut self, extra_cpus_count: usize) {
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
                    let r = &*self.bsp_cpu_local.as_ptr();
                    self.cpu_locals_ptr.as_mut()[0] = r;
                } else {
                    self.cpu_locals_ptr.as_mut()[i] = &self.smp_cpu_locals.as_ref()[i - 1];
                }
            }
        }
    }
}

static CPU_LOCAL_CONTAINER: SpinLock<CPULocalsContainer> = SpinLock::new(CPULocalsContainer::new());

/// By default we can only hold one CPU Local storage and we don't allocate
///
/// but if SMP was detected this function has to be called with the amount of extra CPUs to reserve space for, once the allocator is initialized.
pub fn reserve_cpus(cpus_count: usize) {
    if cpus_count != 0 {
        let mut l = CPU_LOCAL_CONTAINER.lock();
        l.reserve(cpus_count);
    }
}

/// Safely allocate a place for storing CPULocals
///
/// [`reserve_cpus`] must be called if you want to allocate more than 1 CPU, if you attempt to allocate to much CPU Locals, this will panic
pub fn allocate_cpu_local(local: CPULocal) -> &'static mut CPULocal {
    unsafe { CPU_LOCAL_CONTAINER.lock().insert_next(local).as_mut() }
}

/// Gets references to all allocated CPU Locals
pub fn get_all_cpu_locals() -> &'static [&'static CPULocal] {
    unsafe { CPU_LOCAL_CONTAINER.lock().cpu_locals() }
}

impl CPULocal {
    pub fn get_all() -> &'static [&'static CPULocal] {
        get_all_cpu_locals()
    }
}
