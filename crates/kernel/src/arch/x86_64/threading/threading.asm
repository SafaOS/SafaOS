.global restore_cpu_status_full
.global restore_cpu_status_partial
.global context_switch_stub

.macro restore_cpu_status_inner
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

    mov rax, [rdi + 0x98]
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
    call context_switch
    // UNREACHABLE!!!
    ud2
