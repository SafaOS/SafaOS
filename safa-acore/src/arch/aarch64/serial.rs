use crate::misc::VirtAddr;
use core::cell::SyncUnsafeCell;
use core::ptr::NonNull;

static QEMU_SERIAL_ADDR: SyncUnsafeCell<VirtAddr> =
    SyncUnsafeCell::new(VirtAddr::new(0x9000000 + 0xffff000000000000));

pub type SerialInner = VirtAddr;
pub const fn new_serial() -> SerialInner {
    VirtAddr::null()
}

pub fn init_serial(_serial: &mut SerialInner) -> Result<(), &'static str> {
    todo!("Initialize serial")
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
