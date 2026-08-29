use crate::arch::inner;

pub use inner::boot::kboot;

/// Initializes the architecture without alloc/VMM proper memory mapping.
///
/// This is ran before even the PMM is initialized.
pub fn init_phase1() {
    inner::boot::init_phase1()
}
