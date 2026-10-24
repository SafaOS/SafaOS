unsafe extern "C" {
    fn __percpu_cpu_index() -> usize;
}

#[inline(always)]
pub fn cpu_index() -> usize {
    unsafe { __percpu_cpu_index() }
}
