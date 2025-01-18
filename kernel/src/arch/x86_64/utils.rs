use core::arch::asm;

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
            let ebx: u32;
            let edx: u32;
            let ecx: u32;
            let mut vendor_id = [0u32; 3];

            asm!(
                "
            cpuid
            mov eax, ebx
            ",
                in("eax") 0,
                lateout("eax") ebx,
                lateout("ecx") ecx,
                lateout("edx") edx,
            );

            vendor_id[0] = ebx;
            vendor_id[1] = edx;
            vendor_id[2] = ecx;

            let vendor_id: [u8; 12] = core::mem::transmute(vendor_id);
            let vendor_id = heapless::Vec::from_slice(&vendor_id).unwrap_unchecked();

            heapless::String::from_utf8_unchecked(vendor_id).into()
        }
    }

    fn fetch_model() -> HeaplessString<48> {
        unsafe {
            let mut model: [u8; 48] = [0u8; 48];

            for i in 0..3 {
                let eax: u32;
                let ebx: u32;
                let ecx: u32;
                let edx: u32;

                asm!(
                    "
            cpuid
            mov esi, ebx
            ",
                    in("eax") 0x80000002 + i,
                    lateout("eax") eax,
                    lateout("esi") ebx,
                    lateout("ecx") ecx,
                    lateout("edx") edx,
                );

                let index = i * 16;
                model[index..index + 4].copy_from_slice(&eax.to_le_bytes());
                model[index + 4..index + 8].copy_from_slice(&ebx.to_le_bytes());
                model[index + 8..index + 12].copy_from_slice(&ecx.to_le_bytes());
                model[index + 12..index + 16].copy_from_slice(&edx.to_le_bytes());
            }

            let model = heapless::Vec::from_slice(&model).unwrap_unchecked();
            heapless::String::from_utf8_unchecked(model).into()
        }
    }

    fn fetch_address_space() -> (u8, u8) {
        unsafe {
            let mut eax: u32;

            asm!(
                "
            cpuid
            ",
                in("eax") 0x80000008u32,
                lateout("eax") eax,
            );

            ((eax & 0xFF) as u8, ((eax >> 8) & 0xFF) as u8)
        }
    }

    fn fetch_easter_egg() -> Option<HeaplessString<16>> {
        unsafe {
            let eax: u32;
            let ebx: u32;
            let ecx: u32;
            let edx: u32;

            asm!(
                "
            cpuid
            mov edi, ebx
            ",
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
                "
            cpuid
            ",
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
