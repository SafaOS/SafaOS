use crate::misc::PhysAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A [`MemoryRegion`] type.
pub enum MemoryType {
    Usable,
    /// Reserved for unknown puroposes.
    Reserved,
    /// ACPI memory that can be reclaimed after use.
    ACPIReclaimable,
    /// ACPI non-volatile storage.
    ACPINvs,
    /// Memory that is bad or corrupted.
    Bad,
    /// Memory that can be reclaimed from the bootloader once all of the bootloader's resources are used.
    BootloaderReclaimable,
    /// Kernel executable and everything that was requested to be loaded.
    Exe,
    /// Framebuffer.
    Framebuffer,
    Other,
}

impl MemoryType {
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Usable)
    }
}

#[derive(Debug, Clone, Copy)]
/// A memory region in the bootloader's memory map.
pub struct MemoryRegion {
    pub base: PhysAddr,
    pub size: usize,
    pub kind: MemoryType,
}

#[inline(always)]
/// Returns an iterator over the bootloader memory map regopms [`MemoryRegion`].
pub fn memory_map() -> impl Iterator<Item = MemoryRegion> {
    super::current::memory_map()
}
