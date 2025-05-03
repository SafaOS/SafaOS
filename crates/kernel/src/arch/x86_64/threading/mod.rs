pub const STACK_SIZE: usize = PAGE_SIZE * 8;
pub const STACK_START: usize = 0x00007A3000000000;
pub const STACK_END: usize = STACK_START + STACK_SIZE;

pub const RING0_STACK_START: usize = 0x00007A0000000000;
pub const RING0_STACK_END: usize = RING0_STACK_START + STACK_SIZE;

pub const ENVIRONMENT_START: usize = 0x00007E0000000000;
pub const ARGV_START: usize = ENVIRONMENT_START + 0xA000000000;
pub const ENVIRONMENT_VARIABLES_START: usize = ENVIRONMENT_START + 0xE000000000;

pub const ABI_STRUCTURES_START: usize = ENVIRONMENT_START + 0x1000000000;

use core::{arch::global_asm, ptr::NonNull};

use bitflags::bitflags;

use crate::{
    memory::{
        copy_to_userspace, frame_allocator,
        paging::{EntryFlags, MapToError, Page, PhysPageTable, PAGE_SIZE},
    },
    threading::swtch,
    VirtAddr,
};

use super::gdt::{KERNEL_CODE_SEG, KERNEL_DATA_SEG, USER_CODE_SEG, USER_DATA_SEG};

bitflags! {
    #[derive(Default, Debug, Clone, Copy)]
    #[repr(C)]
    pub struct RFLAGS: u64 {
        const ID = 1 << 21;
        const VIRTUAL_INTERRUPT_PENDING = 1 << 20;
        const VIRTUAL_INTERRUPT = 1 << 19;
        const ALIGNMENT_CHECK = 1 << 18;
        const VIRTUAL_8086_MODE = 1 << 17;

        const RESUME_FLAG = 1 << 16;
        const NESTED_TASK = 1 << 14;

        const IOPL_HIGH = 1 << 13;
        const IOPL_LOW = 1 << 12;

        const OVERFLOW_FLAG = 1 << 11;
        const DIRECTION_FLAG = 1 << 10;

        const INTERRUPT_FLAG = 1 << 9;
        const TRAP_FLAG = 1 << 8;

        const SIGN_FLAG = 1 << 7;
        const ZERO_FLAG = 1 << 6;
        const AUXILIARY_CARRY_FLAG = 1 << 4;

        const PARITY_FLAG = 1 << 2;
        const CARRY_FLAG = 1;
    }
}

/// The CPU Status for each thread (registers)
#[derive(Debug, Clone, Copy, Default)]
#[repr(C, packed)]
pub struct CPUStatus {
    rsp: u64,
    rflags: RFLAGS,
    ss: u64,
    cs: u64,

    rip: u64,

    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,

    rbp: u64,
    rdi: u64,
    rsi: u64,

    rdx: u64,
    rcx: u64,
    rbx: u64,
    cr3: u64,
    rax: u64,

    // ffi-safe alternative for u128
    xmm15: [u8; 16],
    xmm14: [u8; 16],
    xmm13: [u8; 16],
    xmm12: [u8; 16],
    xmm11: [u8; 16],
    xmm10: [u8; 16],
    xmm9: [u8; 16],
    xmm8: [u8; 16],
    xmm7: [u8; 16],
    xmm6: [u8; 16],
    xmm5: [u8; 16],
    xmm4: [u8; 16],
    xmm3: [u8; 16],
    xmm2: [u8; 16],
    xmm1: [u8; 16],
    xmm0: [u8; 16],
}

use safa_utils::abi::raw::{processes::AbiStructures, RawSlice};

fn map_byte_slices(
    page_table: &mut PhysPageTable,
    slices: &[&[u8]],
    map_start_addr: usize,
) -> Result<Option<NonNull<RawSlice<u8>>>, MapToError> {
    if slices.is_empty() {
        return Ok(None);
    }

    let mut allocated_bytes_remaining = 0;
    let mut current_page = map_start_addr;

    let mut map_next = |page_table: &mut PhysPageTable, allocated_bytes_remaining: &mut usize| {
        let results = page_table.map_to(
            Page::containing_address(current_page),
            frame_allocator::allocate_frame().ok_or(MapToError::FrameAllocationFailed)?,
            EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
        );
        *allocated_bytes_remaining += 4096;
        current_page += 4096;
        results
    };

    let mut map_next_bytes = |bytes: usize,
                              page_table: &mut PhysPageTable,
                              allocated_bytes_remaining: &mut usize|
     -> Result<(), MapToError> {
        let pages = (bytes + PAGE_SIZE - 1) / PAGE_SIZE;

        for _ in 0..pages {
            map_next(page_table, allocated_bytes_remaining)?;
        }
        Ok(())
    };

    macro_rules! map_if_not_enough {
        ($bytes: expr) => {
            if allocated_bytes_remaining < $bytes {
                map_next_bytes($bytes, page_table, &mut allocated_bytes_remaining)?;
            } else {
                allocated_bytes_remaining -= $bytes;
            }
        };
    }

    const USIZE_BYTES: usize = size_of::<usize>();
    map_if_not_enough!(8);
    let mut start_addr = map_start_addr;
    // argc
    copy_to_userspace(page_table, start_addr, &slices.len().to_ne_bytes());

    // argv*
    start_addr += USIZE_BYTES;

    for slice in slices {
        map_if_not_enough!(slice.len() + 1);

        copy_to_userspace(page_table, start_addr, slice);
        // null-terminate arg
        copy_to_userspace(page_table, start_addr + slice.len(), b"\0");
        start_addr += slice.len() + 1;
    }

    let mut start_addr = start_addr.next_multiple_of(USIZE_BYTES);
    let slices_addr = start_addr;
    let mut current_slice_ptr = map_start_addr + USIZE_BYTES /* after argc */;

    for slice in slices {
        map_if_not_enough!(size_of::<RawSlice<u8>>());

        let raw_slice =
            unsafe { RawSlice::from_raw_parts(current_slice_ptr as *const u8, slice.len()) };
        let bytes: [u8; size_of::<RawSlice<u8>>()] = unsafe { core::mem::transmute(raw_slice) };

        copy_to_userspace(page_table, start_addr, &bytes);
        start_addr += bytes.len();

        current_slice_ptr += slice.len() + 1; // skip the data (and null terminator)
    }

    Ok(Some(unsafe {
        NonNull::new_unchecked(slices_addr as *mut RawSlice<u8>)
    }))
}

/// Maps the arguments to the environment area in the given page table.
/// returns an FFI safe pointer to the argv array
/// returns None if arguments are empty
///
/// # Layout
/// directly at `ARGV_START` is the argv length,
/// followed by the argv raw bytes ([u8]),
/// followed by the argv pointers (RawSlice<u8>).
///
/// the returned slice is a slice of the argv pointers, meaning it is not available until the page table is loaded
/// there is an added null character at the end of each argument for compatibility with C
fn map_argv(
    page_table: &mut PhysPageTable,
    args: &[&str],
) -> Result<Option<NonNull<RawSlice<u8>>>, MapToError> {
    return map_byte_slices(
        page_table,
        unsafe { core::mem::transmute(args) },
        ARGV_START,
    );
}

impl CPUStatus {
    pub fn at(&self) -> VirtAddr {
        self.rip as VirtAddr
    }

    pub fn stack_at(&self) -> VirtAddr {
        self.rsp as VirtAddr
    }

    /// Initializes a new userspace `CPUStatus` instance, initializes the stack, argv, etc...
    /// argument `userspace` determines if the process is in ring0 or not
    /// # Safety
    /// The caller must ensure `page_table` is not freed, as long as [`Self`] is alive otherwise it will cause UB
    pub unsafe fn create(
        page_table: &mut PhysPageTable,
        argv: &[&str],
        env: &[&[u8]],
        structures: AbiStructures,
        entry_point: usize,
        userspace: bool,
    ) -> Result<Self, MapToError> {
        // allocate the stack
        page_table.alloc_map(
            STACK_START,
            STACK_END,
            EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
        )?;

        // allocate the syscall stack
        page_table.alloc_map(
            RING0_STACK_START,
            RING0_STACK_END,
            EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
        )?;

        let argc = argv.len();
        let envc = env.len();

        let argv_ptr = map_argv(page_table, argv)?;
        let argv_ptr = argv_ptr
            .map(|p| p.as_ptr())
            .unwrap_or(core::ptr::null_mut());

        let env_ptr = map_byte_slices(page_table, env, ENVIRONMENT_VARIABLES_START)?;
        let env_ptr = env_ptr.map(|p| p.as_ptr()).unwrap_or(core::ptr::null_mut());

        // ABI structures are structures that are passed to tasks by the kernel
        // currently only stdio is passed
        let structures_bytes: &[u8] =
            &unsafe { core::mem::transmute::<_, [u8; size_of::<AbiStructures>()]>(structures) };

        page_table.alloc_map(
            ABI_STRUCTURES_START,
            ABI_STRUCTURES_START + PAGE_SIZE,
            EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE | EntryFlags::PRESENT,
        )?;
        copy_to_userspace(page_table, ABI_STRUCTURES_START, structures_bytes);

        let abi_structures_ptr = ABI_STRUCTURES_START as *const AbiStructures;

        let (cs, ss, rflags) = if userspace {
            (
                USER_CODE_SEG as u64,
                USER_DATA_SEG as u64,
                RFLAGS::IOPL_LOW | RFLAGS::IOPL_HIGH | RFLAGS::from_bits_retain(0x202),
            )
        } else {
            (
                KERNEL_CODE_SEG as u64,
                KERNEL_DATA_SEG as u64,
                RFLAGS::from_bits_retain(0x202),
            )
        };

        Ok(Self {
            rflags,
            rip: entry_point as u64,
            rdi: argc as u64,
            rsi: argv_ptr as u64,
            rdx: envc as u64,
            rcx: env_ptr as u64,
            r8: abi_structures_ptr as u64,
            cr3: page_table.phys_addr() as u64,
            rsp: STACK_END as u64,
            cs,
            ss,
            ..Default::default()
        })
    }
}

global_asm!(
    "
.global restore_cpu_status
.global context_switch_stub

restore_cpu_status:
    // push the iretq frame
    push [rdi + 16]     // push ss
    push [rdi]          // push rsp
    push [rdi + 8]      // push rflags
    push [rdi + 24]     // push cs
    push [rdi + 32]     // push rip


    mov r15, [rdi + 40]
    mov r14, [rdi + 48]
    mov r13, [rdi + 56]
    mov r12, [rdi + 64]
    mov r11, [rdi + 72]
    mov r10, [rdi + 80]
    mov r9, [rdi + 88]
    mov r8, [rdi + 96]

    mov rbp, [rdi + 104]
    mov rsi, [rdi + 120]

    mov rdx, [rdi + 128]
    mov rcx, [rdi + 136]
    mov rbx, [rdi + 144]

    push [rdi + 0x70] // rdi
    push [rdi + 0xA0] // rax

    lea rax, [rdi + 0xA8]
    movdqu xmm15, [rax+0x00]
    movdqu xmm14, [rax+0x10]
    movdqu xmm13, [rax+0x20]
    movdqu xmm12, [rax+0x30]
    movdqu xmm11, [rax+0x40]
    movdqu xmm10, [rax+0x50]
    movdqu xmm9, [rax+0x60]
    movdqu xmm8, [rax+0x70]
    movdqu xmm7, [rax+0x80]
    movdqu xmm6, [rax+0x90]
    movdqu xmm5, [rax+0xA0]
    movdqu xmm4, [rax+0xB0]
    movdqu xmm3, [rax+0xC0]
    movdqu xmm2, [rax+0xD0]
    movdqu xmm1, [rax+0xE0]
    movdqu xmm0, [rax+0xF0]

    mov rax, [rdi + 0x98]
    mov cr3, rax

    pop rax
    pop rdi

    iretq

context_switch_stub:
    sub rsp, 16*16      // allocate space for xmm registers
    movdqu [rsp+0x00], xmm0
    movdqu [rsp+0x10], xmm1
    movdqu [rsp+0x20], xmm2
    movdqu [rsp+0x30], xmm3
    movdqu [rsp+0x40], xmm4
    movdqu [rsp+0x50], xmm5
    movdqu [rsp+0x60], xmm6
    movdqu [rsp+0x70], xmm7
    movdqu [rsp+0x80], xmm8
    movdqu [rsp+0x90], xmm9
    movdqu [rsp+0xA0], xmm10
    movdqu [rsp+0xB0], xmm11
    movdqu [rsp+0xC0], xmm12
    movdqu [rsp+0xD0], xmm13
    movdqu [rsp+0xE0], xmm14
    movdqu [rsp+0xF0], xmm15

    push rax
    mov rax, cr3
    push rax

    push rbx
    push rcx
    push rdx

    push rsi
    push rdi
    push rbp

    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    push 0    // rip
    push 0x8  // cs
    push 0x10 // ss
    pushfq
    push 0 // rsp
    call context_switch
    // UNREACHABLE!!!
    ud2
"
);

extern "C" {
    ///  Takes a reference to [`CPUStatus`] and sets current cpu status (registers) to it
    pub fn restore_cpu_status(status: &CPUStatus) -> !;
}

extern "x86-interrupt" {
    pub fn context_switch_stub();
}

#[no_mangle]
pub extern "C" fn context_switch(mut capture: CPUStatus, frame: super::interrupts::InterruptFrame) {
    capture.rsp = frame.stack_pointer;
    capture.rip = frame.insturaction;

    capture.cs = frame.code_segment;
    capture.ss = frame.stack_segment;
    capture.rflags = frame.flags;

    unsafe {
        capture = swtch(capture);
        super::interrupts::apic::send_eoi();
        restore_cpu_status(&capture);
    }
}
