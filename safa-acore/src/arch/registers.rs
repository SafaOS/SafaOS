use crate::percpu::CpuLocal;

use super::inner;
pub use inner::registers::ArchCpuID;

#[inline(always)]
/// Returns a reference to the [`CpuLocal`] for this CPU.
pub fn cpu_local() -> &'static CpuLocal {
    inner::registers::cpu_local()
}
