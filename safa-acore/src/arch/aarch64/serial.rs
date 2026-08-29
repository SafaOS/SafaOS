use crate::memory::phys_to_virt;
use crate::misc::{Frame, Page, PhysAddr, VirtAddr};
use crate::paging::PageEntryFlags;
use core::cell::SyncUnsafeCell;
use core::ptr::NonNull;

static QEMU_SERIAL_ADDR: SyncUnsafeCell<VirtAddr> = SyncUnsafeCell::new(VirtAddr::null());

pub type SerialInner = VirtAddr;
pub const fn new_serial() -> SerialInner {
    VirtAddr::null()
}

/// Maps the PL011 QEMU Serial for debug prints before the DTB is parsed in QEMU.
pub fn map_qemu_serial() {
    let phys = PhysAddr::from(0x09000000);
    let virt = phys_to_virt(phys);
    let page = Page::containing(virt);
    let frame = Frame::containing(phys);

    unsafe {
        if crate::paging::PageTable::current()
            .map_to(
                page,
                frame,
                PageEntryFlags::WRITE | PageEntryFlags::DEVICE_UNCACHEABLE,
            )
            .is_ok()
        {
            (*QEMU_SERIAL_ADDR.get()) = virt;

            write_serial_string(&virt, "\nQEMU Serial initialized\n");
        }
    }
}
pub fn init_serial(_serial: &mut SerialInner) -> Result<(), &'static str> {
    Ok(())
}

#[inline(always)]
fn putbyte(ptr: NonNull<u8>, c: u8) {
    if c == b'\n' {
        putbyte(ptr, b'\r');
    }

    unsafe {
        ptr.write_volatile(c);
    }
}

pub fn write_serial_string(inner: &SerialInner, s: &str) {
    let Some(addr) = NonNull::new(inner.into_ptr::<u8>())
        .or_else(|| NonNull::new(unsafe { *QEMU_SERIAL_ADDR.get() }.into_ptr::<u8>()))
    else {
        return;
    };

    for b in s.as_bytes() {
        putbyte(addr, *b);
    }
}
