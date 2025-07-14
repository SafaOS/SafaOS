.section .text
.global restore_cpu_status_full
.global restore_cpu_status_partial
.global context_switch_stub



.equ RING0_RSP_OFFSET, 0x00
.equ RSP_OFFSET, RING0_RSP_OFFSET + 8
.equ RFLAGS_OFFSET, RSP_OFFSET + 8
.equ SS_OFFSET, RFLAGS_OFFSET + 8
.equ CS_OFFSET, SS_OFFSET + 8
.equ RIP_OFFSET, CS_OFFSET + 8


.equ R15_OFFSET, RIP_OFFSET + 8
.equ R14_OFFSET, R15_OFFSET + 8
.equ R13_OFFSET, R14_OFFSET + 8
.equ R12_OFFSET, R13_OFFSET + 8
.equ R11_OFFSET, R12_OFFSET + 8
.equ R10_OFFSET, R11_OFFSET + 8
.equ R9_OFFSET, R10_OFFSET + 8
.equ R8_OFFSET, R9_OFFSET + 8


.equ RBP_OFFSET, R8_OFFSET + 8
.equ RDI_OFFSET, RBP_OFFSET + 8
.equ RSI_OFFSET, RDI_OFFSET + 8
.equ RDX_OFFSET, RSI_OFFSET + 8
.equ RCX_OFFSET, RDX_OFFSET + 8
.equ RBX_OFFSET, RCX_OFFSET + 8

.equ CR3_OFFSET, RBX_OFFSET + 8
.equ RAX_OFFSET, CR3_OFFSET + 8


.equ XMM15_OFFSET, RAX_OFFSET + 8
.equ XMM14_OFFSET, XMM15_OFFSET + 16
.equ XMM13_OFFSET, XMM14_OFFSET + 16
.equ XMM12_OFFSET, XMM13_OFFSET + 16
.equ XMM11_OFFSET, XMM12_OFFSET + 16
.equ XMM10_OFFSET, XMM11_OFFSET + 16
.equ XMM9_OFFSET, XMM10_OFFSET + 16
.equ XMM8_OFFSET, XMM9_OFFSET + 16
.equ XMM7_OFFSET, XMM8_OFFSET + 16
.equ XMM6_OFFSET, XMM7_OFFSET + 16
.equ XMM5_OFFSET, XMM6_OFFSET + 16
.equ XMM4_OFFSET, XMM5_OFFSET + 16
.equ XMM3_OFFSET, XMM4_OFFSET + 16
.equ XMM2_OFFSET, XMM3_OFFSET + 16
.equ XMM1_OFFSET, XMM2_OFFSET + 16
.equ XMM0_OFFSET, XMM1_OFFSET + 16


.macro restore_cpu_status_inner
// push the iretq frame
   push [rdi + SS_OFFSET]     // push ss
   push [rdi + RSP_OFFSET]          // push rsp
   push [rdi + RFLAGS_OFFSET]      // push rflags
   push [rdi + CS_OFFSET]     // push cs
   push [rdi + RIP_OFFSET]     // push rip


   mov r15, [rdi + R15_OFFSET]
   mov r14, [rdi + R14_OFFSET]
   mov r13, [rdi + R13_OFFSET]
   mov r12, [rdi + R12_OFFSET]
   mov r11, [rdi + R11_OFFSET]
   mov r10, [rdi + R10_OFFSET]
   mov r9, [rdi + R9_OFFSET]
   mov r8, [rdi + R8_OFFSET]

   mov rbp, [rdi + RBP_OFFSET]
   mov rsi, [rdi + RSI_OFFSET]

   mov rdx, [rdi + RDX_OFFSET]
   mov rcx, [rdi + RCX_OFFSET]
   mov rbx, [rdi + RBX_OFFSET]

   push [rdi + RDI_OFFSET] // rdi
   push [rdi + RAX_OFFSET] // rax

   lea rax, [rdi + XMM15_OFFSET]
   // TODO: implement lazy FPU initialization
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
.endm

restore_cpu_status_full:
    restore_cpu_status_inner

    mov rax, [rdi + CR3_OFFSET]
    mov cr3, rax

    pop rax
    pop rdi

    iretq

restore_cpu_status_partial:
    restore_cpu_status_inner

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
    // ring0 rsp
    push 0
    call context_switch
    // UNREACHABLE!!!
    ud2
