use core::{arch::asm, cell::SyncUnsafeCell};

use serde::Serialize;
use spin::Lazy;

use crate::utils::HeaplessString;

#[derive(Serialize, Debug)]
pub struct CpuInfo {
    vendor_id: HeaplessString<12>,
    model: HeaplessString<48>,

    physical_address_space: u8,
    virtual_address_space: u8,
    core_count: u8,

    #[serde(skip_serializing_if = "Option::is_none")]
    easter_egg: Option<HeaplessString<16>>,
}

impl CpuInfo {
    fn fetch_vendor_id() -> HeaplessString<12> {
        unsafe {
            let (ebx, ecx, edx): (u32, u32, u32);

            asm!(
                "cpuid",
                "mov eax, ebx",
                in("eax") 0,
                lateout("eax") ebx,
                lateout("ecx") ecx,
                lateout("edx") edx,
            );

            let vendor_id: [u8; 12] = core::mem::transmute([ebx, edx, ecx]);
            let vendor_id = heapless::Vec::from_slice(&vendor_id).unwrap_unchecked();

            heapless::String::from_utf8_unchecked(vendor_id).into()
        }
    }

    fn fetch_model() -> HeaplessString<48> {
        unsafe {
            let mut model: [u8; 48] = [0u8; 48];

            for i in 0..3 {
                let (eax, ebx, ecx, edx): (u32, u32, u32, u32);

                asm!(
                    "cpuid",
                    "mov esi, ebx",
                    in("eax") 0x80000002 + i,
                    lateout("eax") eax,
                    lateout("esi") ebx,
                    lateout("ecx") ecx,
                    lateout("edx") edx,
                );
                let i = i * 16;

                model[i..i + 16]
                    .copy_from_slice(&core::mem::transmute::<_, [u8; 16]>([eax, ebx, ecx, edx]));
            }

            let model = heapless::Vec::from_slice(&model).unwrap_unchecked();
            heapless::String::from_utf8_unchecked(model).into()
        }
    }

    fn fetch_address_space() -> (u8, u8) {
        unsafe {
            let mut eax: u32;

            asm!(
                "cpuid",
                in("eax") 0x80000008u32,
                lateout("eax") eax,
            );

            ((eax & 0xFF) as u8, ((eax >> 8) & 0xFF) as u8)
        }
    }

    fn fetch_easter_egg() -> Option<HeaplessString<16>> {
        unsafe {
            let (eax, ebx, ecx, edx): (u32, u32, u32, u32);

            asm!(
                "cpuid",
                "mov edi, ebx",
                in("eax") 0x8FFFFFFFu32,
                lateout("eax") eax,
                lateout("edi") ebx,
                lateout("ecx") ecx,
                lateout("edx") edx,
            );

            let easter_egg: [u8; 16] = core::mem::transmute([eax, ebx, ecx, edx]);

            if easter_egg[0] == 0 {
                return None;
            }

            let easter_egg = heapless::Vec::from_slice(&easter_egg).ok()?;
            heapless::String::from_utf8(easter_egg)
                .ok()
                .map(HeaplessString::from)
        }
    }

    fn fetch_core_count() -> u8 {
        unsafe {
            let mut eax: u32;

            asm!(
                "cpuid",
                in("eax") 0x4,
                lateout("eax") eax,
            );

            ((eax >> 26) & 0xFF) as u8 + 1
        }
    }
    pub fn fetch() -> Self {
        let (physical_address_space, virtual_address_space) = Self::fetch_address_space();
        Self {
            vendor_id: Self::fetch_vendor_id(),
            model: Self::fetch_model(),
            physical_address_space,
            virtual_address_space,
            core_count: Self::fetch_core_count(),
            easter_egg: Self::fetch_easter_egg(),
        }
    }
}

pub static CPU_INFO: Lazy<CpuInfo> = Lazy::new(|| CpuInfo::fetch());

pub static TICKS_PER_MS: SyncUnsafeCell<u64> = SyncUnsafeCell::new(1);
pub static APIC_TIMER_TICKS_PER_MS: SyncUnsafeCell<u64> = SyncUnsafeCell::new(1);

#[inline(always)]
/// Returns the number of clock cpu cycles per 1ms
pub fn ticks_per_ms() -> u64 {
    unsafe { core::ptr::read_volatile(TICKS_PER_MS.get()) }
}

#[inline(always)]
// actually used in macros but rust thinks it is unused for some reason
#[allow(unused)]
/// Returns the number of milliseconds since the CPU was started
pub fn time() -> u64 {
    let ticks_per_ms = ticks_per_ms();
    let ticks = unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    };
    ticks / ticks_per_ms
}
