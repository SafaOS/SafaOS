use core::arch::global_asm;

use bitflags::bitflags;

use crate::threading::swtch;

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

#[derive(Debug, Clone, Copy, Default)]
#[repr(C, packed)]
pub struct CPUStatus {
    pub rsp: u64,
    pub rflags: RFLAGS,
    pub ss: u64,
    pub cs: u64,

    pub rip: u64,

    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,

    rbp: u64,
    pub rdi: u64,
    pub rsi: u64,

    rdx: u64,
    rcx: u64,
    rbx: u64,
    pub cr3: u64,
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

impl CPUStatus {
    pub fn at(&self) -> usize {
        self.rip as usize
    }

    pub fn stack_at(&self) -> usize {
        self.rsp as usize
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
