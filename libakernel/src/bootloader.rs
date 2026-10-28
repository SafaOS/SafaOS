use crate::misc::VirtAddr;

unsafe extern "C" {
    static HHDM: VirtAddr;

    fn bootloader_cpu_count() -> usize;
}

#[inline(always)]
pub fn hhdm() -> VirtAddr {
    unsafe { HHDM }
}

#[inline(always)]
pub fn cpu_count() -> usize {
    unsafe { bootloader_cpu_count() }
}
