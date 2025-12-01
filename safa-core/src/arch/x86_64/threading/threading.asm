.section .text
.global restore_cpu_status_full
.global restore_cpu_status_partial
.global context_switch_stub
.global thread_yield_wrapper



.equ FS_BASE_OFFSET, 0x00
.equ RING0_RSP_OFFSET, FS_BASE_OFFSET + 8
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


.equ FLOATING_OFFSET, RAX_OFFSET + 16


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

   // TODO: implement lazy FPU initialization
   fxrstor [rdi + FLOATING_OFFSET]
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
    sub rsp, 8        // alignment for the interrupt frame
    sub rsp, 512      // allocate space for fpu registers
    fxsave [rsp]

    /* alignment */
    push rax

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
    // fs
    push 0
    call context_switch
    ud2


thread_yield_wrapper:
  // the extra 8 bytes for alignment
  sub rsp, 0x2C0

  mov [rsp + RAX_OFFSET], rax

  mov [rsp + RBX_OFFSET], rbx
  mov [rsp + RCX_OFFSET], rcx
  mov [rsp + RDX_OFFSET], rdx

  mov [rsp + RSI_OFFSET], rsi
  mov [rsp + RDI_OFFSET], rdi
  mov [rsp + RBP_OFFSET], rbp

  mov [rsp + R8_OFFSET], r8
  mov [rsp + R9_OFFSET], r9
  mov [rsp + R10_OFFSET], r10
  mov [rsp + R11_OFFSET], r11
  mov [rsp + R12_OFFSET], r12
  mov [rsp + R13_OFFSET], r13
  mov [rsp + R14_OFFSET], r14
  mov [rsp + R15_OFFSET], r15

  mov rax, cr3
  mov [rsp + CR3_OFFSET], rax

  // RSP before the SUB
  lea rax, [rsp + 0x2C0]

  // RIP
  mov rbx, [rax]
  mov [rsp + RIP_OFFSET], rbx

  // RSP before the call
  lea rax, [rax + 8]
  mov [rsp + RSP_OFFSET], rax

  mov rax, cs
  mov [rsp + CS_OFFSET], rax

  mov rax, ss
  mov [rsp + SS_OFFSET], rax

  pushfq
  pop rax

  mov [rsp + RFLAGS_OFFSET], rax

  fxsave [rsp + FLOATING_OFFSET]
  call context_switch_and_return
  ud2
