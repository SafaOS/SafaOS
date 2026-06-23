#![allow(static_mut_refs)]
use core::{arch::asm, cell::SyncUnsafeCell};

use crate::{
    VirtAddr,
    arch::x86_64::{smp::set_gs, threading::STACK_SIZE},
    percpu::{self, CpuLocal},
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
    privilege_stack_table: [VirtAddr; 3],
    reserved_2: u64,
    interrupt_stack_table: [VirtAddr; 7],
    reserved_3: u64,
    reserved_4: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    pub const fn new_zeroed() -> Self {
        Self {
            reserved_1: 0,
            privilege_stack_table: [VirtAddr::null(); 3],
            reserved_2: 0,
            interrupt_stack_table: [VirtAddr::null(); 7],
            reserved_3: 0,
            reserved_4: 0,
            iomap_base: 0,
        }
    }

    fn new(stacks: &[SyncUnsafeCell<Stack>]) -> Self {
        Self {
            reserved_1: 0,
            privilege_stack_table: [VirtAddr::null(); 3],
            reserved_2: 0,
            interrupt_stack_table: core::array::from_fn(|i| {
                let stack = &stacks[i % stacks.len()];
                VirtAddr::from_ptr(stack.get()) + size_of::<Stack>()
            }),
            reserved_3: 0,
            reserved_4: 0,
            iomap_base: 0,
        }
    }
}

#[repr(C, align(16))]
struct Stack([u8; STACK_SIZE]);

const TSS_STACKS_COUNT: usize = 3;
percpu::define! {
    static TSS_STACKS: [SyncUnsafeCell<Stack>; TSS_STACKS_COUNT] = const {
        [const { SyncUnsafeCell::new(Stack([0xFA; STACK_SIZE])) }; TSS_STACKS_COUNT]
    };
}

percpu::define! {
    static TSS: SyncUnsafeCell<TaskStateSegment> = const {
        SyncUnsafeCell::new(TaskStateSegment::new_zeroed())
    };
}

/// Sets the TSS addr for the current CPU
#[inline]
pub unsafe fn set_kernel_tss_stack(stack_end: VirtAddr) {
    let tss = unsafe { &mut *TSS.get() };
    tss.privilege_stack_table[0] = stack_end;
}
/// Gets the TSS addr for the current CPU
#[inline]
pub unsafe fn get_kernel_tss_stack() -> VirtAddr {
    let tss = unsafe { &mut *TSS.get() };
    tss.privilege_stack_table[0]
}

pub type GDTType = [GDTEntry; 7];

percpu::define! {
    static GDT: SyncUnsafeCell<GDTType> = const {
        SyncUnsafeCell::new([GDTEntry::default(); 7])
    };
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

percpu::define! {
    static GDT_DESCRIPTOR: SyncUnsafeCell<GDTDescriptor> = const {
        SyncUnsafeCell::new(unsafe {core::mem::zeroed()})
    };
}

unsafe fn reload_tss() {
    unsafe { asm!("ltr {0:x}", in(reg) TSS_SEG as u16) }
}

pub fn init_gdt(cpu: &'static CpuLocal) {
    let tss = TSS.borrow_for(cpu);
    let gdt = unsafe { &mut *GDT.borrow_for(cpu).get() };

    *gdt = [
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
            ((tss.get() as u64) & 0xFFFFFFFF) as u32,
            (size_of::<TaskStateSegment>() - 1) as u32,
            ACCESS_VALID | ACCESS_TYPE_TSS,
            FLAG_PAGELIMIT | FLAG_LONG,
        ), // TSS segment
        GDTEntry::new_upper_64seg(tss.get() as u64),
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
    ];

    let gdt_descriptor = unsafe { &mut *GDT_DESCRIPTOR.borrow_for(cpu).get() };
    *gdt_descriptor = GDTDescriptor {
        limit: (size_of::<GDTType>() - 1) as u16,
        base: gdt,
    };

    unsafe {
        asm!("lgdt [{}]", in(reg) gdt_descriptor, options(nostack));

        asm!(
            "
            mov gs, ax
            mov fs, ax
            mov ds, ax
            mov es, ax
            mov ss, ax
        ", in("ax") 0x10
        );

        asm!(
            "
            push 0x08
            lea rax, [rip + 2f]
            push rax
            retfq
            2:
            ", out("rax") _,
        );

        set_gs(VirtAddr::from_ptr(cpu as *const CpuLocal));
        *tss.get() = TaskStateSegment::new(&*TSS_STACKS);
        reload_tss();
    }
}
