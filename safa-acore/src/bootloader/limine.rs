//! Implementation of the limine bootloader.

use core::{num::NonZero, ptr::NonNull};

use crate::{
    arch::ArchCpuID,
    bootloader::{MemoryRegion, MemoryType},
    misc::{PhysAddr, VirtAddr},
};
use limine::{
    BaseRevision,
    request::{
        DateAtBootRequest, DtbRequest, ExecutableAddressRequest, ExecutableFileRequest,
        FramebufferRequest, HhdmRequest, MemmapRequest, MpRequest, RsdpRequest,
        TscFrequencyRequest,
    },
};

use crate::bootloader::{FramebufferInfo, PixelFormat};

#[used]
#[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(6);

#[used]
#[unsafe(link_section = ".requests")]
static MP_REQUEST: MpRequest = MpRequest::new(0);

#[used]
#[unsafe(link_section = ".requests")]
static DEVICE_TREE_REQUEST: DtbRequest = DtbRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static KERNEL_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static KERNEL_FILE_REQUEST: ExecutableFileRequest = ExecutableFileRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MMAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static DATE_REQUEST: DateAtBootRequest = DateAtBootRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static TSC_FREQ: TscFrequencyRequest = TscFrequencyRequest::new();

#[inline]
pub fn tsc_freq_hz() -> Option<NonZero<u64>> {
    TSC_FREQ
        .response()
        .and_then(|resp| NonZero::new(resp.frequency))
}

#[inline]
pub fn framebuffer() -> Option<FramebufferInfo> {
    FRAMEBUFFER_REQUEST.response().and_then(|resp| {
        resp.framebuffers().iter().find_map(|f| {
            let fmt = match (
                (f.red_mask_shift, f.red_mask_size),
                (f.green_mask_shift, f.green_mask_size),
                (f.blue_mask_shift, f.blue_mask_size),
            ) {
                ((0, 8), (8, 8), (16, 8)) => Some(PixelFormat::Rgb888),
                ((16, 8), (8, 8), (0, 8)) => Some(PixelFormat::Bgr888),
                _ => None,
            }?;

            Some(FramebufferInfo {
                base: NonNull::new(f.address())?,
                format: fmt,
                height: f.height as u32,
                width: f.width as u32,
                pitch: f.pitch as usize,
                bpp: f.bpp,
            })
        })
    })
}

#[inline]
pub fn hhdm() -> VirtAddr {
    HHDM_REQUEST
        .response()
        .map(|resp| VirtAddr::new(resp.offset as usize))
        .unwrap_or(VirtAddr::null())
}

pub type CpuInfoInner = limine::mp::MpInfo;

pub fn get_extra_arg(inner: &CpuInfoInner) -> Option<u64> {
    Some(inner.extra_argument())
}

pub fn bootstrap(
    cpu_info: &super::CpuInfo,
    f: unsafe extern "C" fn(&CpuInfoInner) -> !,
    extra_arg: u64,
) -> bool {
    cpu_info.inner.bootstrap(f, extra_arg);
    true
}

#[inline(always)]
pub fn cpu_id(inner: &CpuInfoInner) -> ArchCpuID {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        ArchCpuID::from_raw(inner.processor_id as u64)
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        ArchCpuID::from_raw(inner.lapic_id as u64)
    }
}

#[inline(always)]
pub fn cpus() -> Option<&'static [&'static CpuInfoInner]> {
    Some(
        MP_REQUEST
            .response()
            .expect("Failed to get MP response")
            .cpus(),
    )
}

#[inline(always)]
pub fn memory_map() -> impl Iterator<Item = MemoryRegion> {
    MMAP_REQUEST
        .response()
        .expect("Failed to get MMAP response")
        .entries()
        .iter()
        .map(|e| {
            let base = PhysAddr::new(e.base as usize);
            let size = e.length as usize;
            let kind = match e.type_ {
                0 => MemoryType::Usable,
                1 => MemoryType::Reserved,
                2 => MemoryType::ACPIReclaimable,
                3 => MemoryType::ACPINvs,
                4 => MemoryType::Bad,
                5 => MemoryType::BootloaderReclaimable,
                6 => MemoryType::Exe,
                7 => MemoryType::Framebuffer,
                _ => MemoryType::Other,
            };

            MemoryRegion { base, size, kind }
        })
}
