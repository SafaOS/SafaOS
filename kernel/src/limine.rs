use alloc::slice;
use lazy_static::lazy_static;
use limine::file::File;
use limine::framebuffer::MemoryModel;
use limine::modules::InternalModule;
use limine::modules::ModuleFlags;
use limine::request::FramebufferRequest;
use limine::request::HhdmRequest;
use limine::request::KernelAddressRequest;
use limine::request::KernelFileRequest;
use limine::request::MemoryMapRequest;
use limine::request::ModuleRequest;
use limine::request::RsdpRequest;

use limine::response::MemoryMapResponse;
use limine::BaseRevision;

use crate::drivers::framebuffer::FrameBufferInfo;
use crate::drivers::framebuffer::PixelFormat;
use crate::memory::align_up;
use crate::utils::ustar::TarArchiveIter;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[link_section = ".requests"]
static KERNEL_ADDRESS_REQUEST: KernelAddressRequest = KernelAddressRequest::new();

#[used]
#[link_section = ".requests"]
static KERNEL_FILE_REQUEST: KernelFileRequest = KernelFileRequest::new();

#[used]
#[link_section = ".requests"]
static MMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

const RAMDISK_MODULE: InternalModule = InternalModule::new()
    .with_path(c"ramdisk.tar")
    .with_flags(ModuleFlags::REQUIRED);

#[used]
#[link_section = ".requests"]
static MODULES_REQUEST: ModuleRequest =
    ModuleRequest::new().with_internal_modules(&[&RAMDISK_MODULE]);

pub fn get_phy_offset() -> usize {
    HHDM_REQUEST.get_response().unwrap().offset() as usize
}

pub fn rsdp_addr() -> usize {
    RSDP_REQUEST.get_response().unwrap().address() as usize
}

pub fn kernel_file() -> &'static File {
    KERNEL_FILE_REQUEST.get_response().unwrap().file()
}

/// returns addr to the kernel image and it's size
pub fn kernel_image_info() -> (*const u8, usize) {
    let file = kernel_file();
    let size = file.size() as usize;
    let ptr = file.addr();

    (ptr, size)
}

pub fn mmap_request() -> &'static MemoryMapResponse {
    MMAP_REQUEST.get_response().unwrap()
}

lazy_static! {
    pub static ref MEMORY_END: usize = {
        let mut largest_addr = 0;
        for entry in mmap_request().entries() {
            let end = (entry.base + entry.length) as usize;

            if end > largest_addr {
                largest_addr = end;
            }
        }

        core::cmp::max(0x0000_0001_0000_0000, largest_addr)
    };
}

pub fn get_phy_offset_end() -> usize {
    get_phy_offset() + *MEMORY_END
}

pub fn get_framebuffer() -> (&'static mut [u8], FrameBufferInfo) {
    let mut buffers = FRAMEBUFFER_REQUEST.get_response().unwrap().framebuffers();
    let first = buffers.next().unwrap();

    let pixel_format = match first.memory_model() {
        MemoryModel::RGB => PixelFormat::Rgb,
        _ => panic!("unknown limine framebuffer format"),
    };

    let bytes_per_pixel = align_up(first.bpp() as usize, 8) / 8;
    let info = FrameBufferInfo {
        bytes_per_pixel,
        stride: first.pitch() as usize / bytes_per_pixel,
        pixel_format,
    };

    assert_eq!(info.bytes_per_pixel, 4);

    let size = (first.width() * first.height() * first.bpp() as u64 / 8) as usize;
    let buffer = unsafe { slice::from_raw_parts_mut(first.addr(), size) };

    (buffer, info)
}

pub fn get_ramdisk_file() -> &'static File {
    MODULES_REQUEST
        .get_response()
        .expect("failed getting modules!")
        .modules()[0]
}

pub fn get_ramdisk() -> TarArchiveIter<'static> {
    unsafe { TarArchiveIter::new(get_ramdisk_file().addr()) }
}
