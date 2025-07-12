#![allow(static_mut_refs)]
use core::{
    arch::asm,
    cell::SyncUnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use lazy_static::lazy_static;

use crate::{
    VirtAddr,
    arch::x86_64::threading::{STACK_SIZE, arch_cpu_local_storage_ptr},
};

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct GDTEntry {
    limit0: u16,
    base0: u16,
    base1: u8,
    access: u8,
    limit1_flags: u8,
    base2: u8,
}

impl GDTEntry {
    const fn default() -> Self {
        Self {
            limit0: 0,
            base0: 0,
            base1: 0,
            access: 0,
            limit1_flags: 0,
            base2: 0,
        }
    }

    const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Self {
        let mut encoded = Self::default();

        encoded.limit0 = (limit & 0xFFFF) as u16;
        encoded.limit1_flags = ((limit >> 16) & 0x0F) as u8; // third limit byte
        encoded.limit1_flags |= flags & 0xF0; // first 4 bits

        encoded.base0 = (base & 0xFFFF) as u16;
        encoded.base1 = ((base >> 16) & 0xFF) as u8;
        encoded.base2 = ((base >> 24) & 0xFF) as u8;

        encoded.access = access;
        encoded
    }

    const fn new_upper_64seg(base: u64) -> Self {
        let mut encoded = Self::default();
        let base = (base >> 32) as u32;

        encoded.limit0 = (base & 0xFFFF) as u16;
        encoded.base0 = ((base >> 16) & 0xFFFF) as u16;
        encoded
    }
}

// TODO convert to bitflags
const ACCESS_WRITE_READ: u8 = 1 << 1;
const ACCESS_EXECUTABLE: u8 = 1 << 3;
const NON_SYSTEM: u8 = 1 << 4;

const ACCESS_DPL0: u8 = 1 << 5;
const ACCESS_DPL1: u8 = 1 << 6;

const ACCESS_VALID: u8 = 1 << 7;

const ACCESS_TYPE_TSS: u8 = 0x9;

const FLAG_LONG: u8 = 1 << 5;
const FLAG_PAGELIMIT: u8 = 1 << 7;

// TSS setup
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct TaskStateSegment {
    reserved_1: u32,
    pub privilege_stack_table: [u64; 3],
    reserved_2: u64,
    pub interrupt_stack_table: [u64; 7],
    reserved_3: u64,
    reserved_4: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    pub const fn new() -> Self {
        Self {
            reserved_1: 0,
            privilege_stack_table: [0u64; 3],
            reserved_2: 0,
            interrupt_stack_table: [0u64; 7],
            reserved_3: 0,
            reserved_4: 0,
            iomap_base: 0,
        }
    }
}

const MAX_GDT_COUNT: usize = 256;
static TSS_STACKS: [SyncUnsafeCell<[u8; STACK_SIZE]>; MAX_GDT_COUNT * 2] =
    [const { SyncUnsafeCell::new([0xAA; STACK_SIZE]) }; MAX_GDT_COUNT * 2];

lazy_static! {
    static ref TSS: [SyncUnsafeCell<TaskStateSegment>; MAX_GDT_COUNT] = core::array::from_fn(|n| {
        let mut tss = TaskStateSegment::new();

        tss.interrupt_stack_table[0] = TSS_STACKS[(n * 2) + 0].get() as u64;
        tss.interrupt_stack_table[1] = TSS_STACKS[(n * 2) + 1].get() as u64;
        tss.privilege_stack_table[0] = 0;

        SyncUnsafeCell::new(tss)
    });
}

/// Gets the TSS addr for the current CPU
pub unsafe fn set_kernel_tss_stack(stack_end: VirtAddr) {
    unsafe {
        let cpu_local = &*arch_cpu_local_storage_ptr();
        let tss = cpu_local.tss_ptr;
        (*tss).privilege_stack_table[0] = stack_end.into_raw() as u64;
    }
}

pub type GDTType = [GDTEntry; 7];
lazy_static! {
static ref GDTS: [GDTType; MAX_GDT_COUNT] = core::array::from_fn(|index| [
    GDTEntry::default(),
    GDTEntry::new(
        0,
        0xFFFFF,
        ACCESS_VALID | NON_SYSTEM | ACCESS_WRITE_READ | ACCESS_EXECUTABLE,
        FLAG_PAGELIMIT | FLAG_LONG,
    ), // kernel code segment
    GDTEntry::new(
        0,
        0xFFFFF,
        ACCESS_VALID | ACCESS_WRITE_READ | NON_SYSTEM,
        FLAG_PAGELIMIT | FLAG_LONG,
    ), // kernel data segment
    GDTEntry::new(
        ((TSS[index].get() as u64) & 0xFFFFFFFF) as u32,
        (size_of::<TaskStateSegment>() - 1) as u32,
        ACCESS_VALID | ACCESS_TYPE_TSS,
        FLAG_PAGELIMIT | FLAG_LONG,
    ), // TSS segment
    GDTEntry::new_upper_64seg(TSS[index].get() as u64),
    GDTEntry::new(
        0,
        0xFFFFF,
        ACCESS_VALID
            | NON_SYSTEM
            | ACCESS_DPL0
            | ACCESS_DPL1
            | ACCESS_WRITE_READ
            | ACCESS_EXECUTABLE,
        FLAG_PAGELIMIT | FLAG_LONG,
    ), // user code segment
    GDTEntry::new(
        0,
        0xFFFFF,
        ACCESS_VALID | NON_SYSTEM | ACCESS_DPL0 | ACCESS_DPL1 | ACCESS_WRITE_READ,
        FLAG_PAGELIMIT | FLAG_LONG,
    ), // user data segment
]);
}

pub const KERNEL_CODE_SEG: u8 = (1 * 8) | 0;
pub const KERNEL_DATA_SEG: u8 = (2 * 8) | 0;
pub const TSS_SEG: u8 = (3 * 8) | 3;

pub const USER_CODE_SEG: u8 = (5 * 8) | 3;
pub const USER_DATA_SEG: u8 = (6 * 8) | 3;

#[repr(C, packed)]
pub struct GDTDescriptor {
    limit: u16,
    base: *const GDTType,
}

unsafe impl Send for GDTDescriptor {}
unsafe impl Sync for GDTDescriptor {}

lazy_static! {
    static ref GDT_DESCRIPTORS: [GDTDescriptor; MAX_GDT_COUNT] = {
        let mut descriptors = [const { unsafe { core::mem::zeroed() } }; MAX_GDT_COUNT];
        let mut i = 0;
        for gdt in &*GDTS {
            descriptors[i] = GDTDescriptor {
                limit: (size_of::<GDTType>() - 1) as u16,
                base: gdt,
            };

            i += 1;
        }
        descriptors
    };
}

static NEXT_GDT_DESCRIPTOR: AtomicUsize = AtomicUsize::new(0);

unsafe fn reload_tss() {
    unsafe { asm!("ltr {0:x}", in(reg) TSS_SEG as u16) }
}

lazy_static! {
    /// Pointer to the TSS of CPU 0
    /// because CPU 0 boots in a special way
    pub static ref TSS0_PTR: usize = TSS[0].get() as usize;
}

#[must_use = "returns a pointer to the TSS of the current CPU, this pointer must be stored in the CPU Local Storage"]
pub fn init_gdt() -> *mut TaskStateSegment {
    let this_gdt_index = NEXT_GDT_DESCRIPTOR.fetch_add(1, Ordering::SeqCst);
    let gdt_descriptor: &GDTDescriptor = &GDT_DESCRIPTORS[this_gdt_index];

    unsafe {
        asm!("lgdt [{}]", in(reg) gdt_descriptor, options(nostack));

        asm!(
            "
            mov ax, 0x10
            mov gs, ax
            mov fs, ax
            mov ds, ax
            mov es, ax
            mov ss, ax
        "
        );

        asm!(
            "
            push 0x08
            lea rax, [rip + 2f]
            push rax
            retfq
            2:
            ",
            options(nostack),
        );

        reload_tss();
        TSS[this_gdt_index].get()
    }
}
