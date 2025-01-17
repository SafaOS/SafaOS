use core::arch::asm;

use serde::Serialize;

pub struct VendorId(heapless::String<12>);
impl Serialize for VendorId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.as_str().trim_matches('\0'))
    }
}

impl From<heapless::String<12>> for VendorId {
    fn from(s: heapless::String<12>) -> Self {
        Self(s)
    }
}

pub struct CpuModel(heapless::String<48>);
impl Serialize for CpuModel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.as_str().trim_matches('\0'))
    }
}

#[derive(Serialize)]
pub struct CpuInfo {
    vendor_id: VendorId,
    model: CpuModel,
}

impl CpuInfo {
    fn fetch_vendor_id() -> VendorId {
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

    fn fetch_model() -> CpuModel {
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
            CpuModel(heapless::String::from_utf8_unchecked(model))
        }
    }

    pub fn fetch() -> Self {
        Self {
            vendor_id: Self::fetch_vendor_id(),
            model: Self::fetch_model(),
        }
    }
}
