use core::cell::SyncUnsafeCell;
use core::fmt::{self, Write};

use crate::arch::paging::current_higher_root_table;
use crate::memory::frame_allocator::Frame;
use crate::memory::paging::{PAGE_SIZE, Page, PageEntryFlags, PageTableOps};
use crate::memory::vmm::{VMMMFlags, VirtualMemoryManager};
use crate::utils::locks::SpinLock;
use crate::{PhysAddr, VirtAddr};

// hack to allow debug prints before the DTB is parsed in QEMU
pub static PL011: SyncUnsafeCell<Option<VirtAddr>> = SyncUnsafeCell::new(None);

/// Maps the PL011 QEMU Serial for debug prints before the DTB is parsed in QEMU.
pub fn init_serial_qemu() {
    let phys = PhysAddr::from(0x9000000);
    let virt = phys.into_virt();
    let page = Page::containing(virt);
    let frame = Frame::containing_address(phys);

    unsafe {
        if current_higher_root_table()
            .map_range(
                Page::iter_pages(page, page.next()),
                Frame::iter_frames(frame, Frame::containing_address(phys + PAGE_SIZE)),
                PageEntryFlags::WRITE,
            )
            .is_ok()
        {
            *PL011.get() = Some(virt);
        }
    }
}
pub unsafe fn map_serial(vmm: &mut VirtualMemoryManager) {
    let phys_addr = *super::cpu::PL011BASE;
    let virt_addr = vmm
        .map_direct_phys(&"PL011", None, phys_addr, 1, VMMMFlags::WRITEABLE)
        .expect("Failed to map Serial");

    unsafe { *PL011.get() = Some(virt_addr) }
}

#[inline(always)]
fn putbyte(c: u8) {
    if c == b'\n' {
        putbyte(b'\r');
    }

    unsafe {
        if let Some(virt_addr) = *PL011.get() {
            virt_addr.into_ptr::<u8>().write_volatile(c);
        }
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
