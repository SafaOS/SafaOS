//! FIXME: Hardcoded to qemu values, use device trees.

use crate::{
    PhysAddr, VirtAddr,
    memory::{
        frame_allocator::Frame,
        paging::PAGE_SIZE,
        vmm::{self, VMMMFlags},
    },
    utils::locks::LazyLock,
};
static IO_PHYS_BASE: PhysAddr = PhysAddr::from(0x3eff0000);
static IO_SPACE_SIZE: usize = 0x10000;

static IO_SPACE_BASE: LazyLock<VirtAddr> = LazyLock::new(|| allocate_io_space());
pub fn allocate_io_space() -> VirtAddr {
    vmm::with_root(|vmm| {
        vmm.map_direct(
            &"IO_SPACE",
            None,
            IO_SPACE_SIZE,
            VMMMFlags::UNCACHABLE | VMMMFlags::WRITEABLE,
            (0..IO_SPACE_SIZE.div_ceil(PAGE_SIZE))
                .map(|i| IO_PHYS_BASE + (i * PAGE_SIZE))
                .map(|p| Frame::containing_address(p)),
        )
    })
    .expect("Failed to allocate IO Space")
}

fn port_ptr<T>(port: u16) -> *mut T {
    let port = port as usize;
    assert!(port + size_of::<T>() < IO_SPACE_SIZE, "port too big");

    unsafe { IO_SPACE_BASE.into_ptr::<T>().byte_add(port) }
}

#[allow(unused)]
pub unsafe fn inb(port: u16) -> u8 {
    unsafe { port_ptr::<u8>(port).read_volatile() }
}
#[allow(unused)]
pub unsafe fn outb(port: u16, value: u8) {
    unsafe { port_ptr::<u8>(port).write_volatile(value) }
}

#[allow(unused)]
pub unsafe fn outw(port: u16, value: u16) {
    unsafe { port_ptr::<u16>(port).write_volatile(value) }
}

#[allow(unused)]
pub unsafe fn outl(port: u16, value: u32) {
    unsafe { port_ptr::<u32>(port).write_volatile(value) }
}

#[allow(unused)]
pub unsafe fn inw(port: u16) -> u16 {
    unsafe { port_ptr::<u16>(port).read_volatile() }
}

#[allow(unused)]
pub unsafe fn inl(port: u16) -> u32 {
    unsafe { port_ptr::<u32>(port).read_volatile() }
}
