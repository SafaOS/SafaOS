use crate::{
    limine::MEMORY_SIZE,
    memory::{frame_allocator, paging::PAGE_SIZE},
    threading::{self},
};

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SysInfo {
    pub total_mem: usize,
    pub used_mem: usize,
    pub processes_count: usize,
}

#[no_mangle]
pub fn info(sysinfo: &mut SysInfo) {
    let used_mem = frame_allocator::memory_mapped() * PAGE_SIZE;

    *sysinfo = SysInfo {
        total_mem: *MEMORY_SIZE,
        used_mem,
        processes_count: threading::pcount(),
    }
}
