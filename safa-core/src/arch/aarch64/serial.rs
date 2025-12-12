use core::cell::SyncUnsafeCell;
use core::fmt::{self, Write};

use crate::memory::vmm::{VMMMFlags, VirtualMemoryManager};
use crate::utils::locks::SpinLock;
use crate::{PhysAddr, VirtAddr};

// hack to allow debug prints before the DTB is parsed in QEMU
pub static PL011: SyncUnsafeCell<VirtAddr> =
    SyncUnsafeCell::new(PhysAddr::from(0x09000000).into_virt());

pub unsafe fn map_serial(vmm: &mut VirtualMemoryManager) {
    let phys_addr = *super::cpu::PL011BASE;
    let virt_addr = vmm
        .map_direct_phys(&"PL011", None, phys_addr, 1, VMMMFlags::WRITEABLE)
        .expect("Failed to map Serial");

    unsafe { *PL011.get() = virt_addr }
}

#[inline(always)]
fn putbyte(c: u8) {
    if c == b'\n' {
        putbyte(b'\r');
    }

    unsafe {
        (*PL011.get()).into_ptr::<u8>().write_volatile(c);
    };
}

fn putc(c: char) {
    // FIXME: utf8?
    putbyte(c as u8);
}

pub(super) fn write_str(s: &str) {
    for c in s.chars() {
        putc(c);
    }
}

pub struct Serial;
/// Global Serial writer
pub static SERIAL: SpinLock<Serial> = SpinLock::new(Serial);

impl Write for Serial {
    fn write_char(&mut self, c: char) -> fmt::Result {
        putc(c);
        Ok(())
    }

    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

pub fn _serial(args: fmt::Arguments) {
    super::without_interrupts(|| SERIAL.lock().write_fmt(args).unwrap())
}
